//! PumpSwap AMM: `Pool` account decoding and constant-product quote math.
//!
//! Program ID and the `Pool` account layout come from the official IDL
//! (`idl/pump_amm.json`, vendored from `pump-fun/pump-public-docs`).
//! Unlike Pump's bonding curve, the real reserves this AMM prices against
//! live in two separate SPL token accounts (`pool_base_token_account`,
//! `pool_quote_token_account`) — the `Pool` struct itself does not hold
//! them, so callers must fetch those two balances themselves and pass them
//! in; this module never reads accounts.
//!
//! Both directions were verified against real mainnet `BuyEvent`/`SellEvent`
//! logs on non-boost pools (see `tests/amm_test.rs`):
//! - sell: `gross_quote_out = floor(base_in * quote_reserves / (base_reserves + base_in))`,
//!   matching the real gross output exactly (not just within rounding).
//! - buy (`buy_exact_quote_in`): `base_out = floor(base_reserves - k / (quote_reserves + net_quote_in))`,
//!   matching the real base output to within 1 unit of integer truncation.
//!
//! Scope: only `is_mayhem_mode = false`, `is_cashback_coin = false`, and
//! `virtual_quote_reserves = 0` ("non-boost") pools are priced. A real
//! `buy_exact_quote_in` trade on a boosted pool (`virtual_quote_reserves`
//! nonzero) was ~9% off from this formula when tested — confirming boosted
//! pools use different, not-yet-understood pricing and must not be priced
//! here; `is_standard` refuses them.
//!
//! Fees (`lp_fee`, `protocol_fee`, `coin_creator_fee`) are deliberately not
//! computed here, for the same reason as Pump: a real trade showed
//! `coin_creator_fee` can be zero even when its bps rate is nonzero
//! elsewhere, which this module cannot predict from local state alone.
//! `quote_buy`/`quote_sell` return the gross, fee-free swap amount; replay
//! must read the real fee fields off the historical event instead.

use solana_pubkey::Pubkey;

pub const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const POOL_LEN: usize = 8 + 1 + 2 + 32 * 7 + 8 + 1 + 1 + 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    DiscriminatorMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub virtual_quote_reserves: i128,
}

impl Pool {
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < POOL_LEN {
            return Err(DecodeError::TooShort);
        }
        if data[..8] != POOL_DISCRIMINATOR {
            return Err(DecodeError::DiscriminatorMismatch);
        }
        let mut r = Reader::new(&data[8..]);
        Ok(Pool {
            pool_bump: r.u8(),
            index: r.u16(),
            creator: r.pubkey(),
            base_mint: r.pubkey(),
            quote_mint: r.pubkey(),
            lp_mint: r.pubkey(),
            pool_base_token_account: r.pubkey(),
            pool_quote_token_account: r.pubkey(),
            lp_supply: r.u64(),
            coin_creator: r.pubkey(),
            is_mayhem_mode: r.bool(),
            is_cashback_coin: r.bool(),
            virtual_quote_reserves: r.i128(),
        })
    }

    /// Pool variants this Stage 1 math is verified for — see module docs
    /// for why boosted pools (`virtual_quote_reserves != 0`) are refused.
    pub fn is_standard(&self) -> bool {
        !self.is_mayhem_mode && !self.is_cashback_coin && self.virtual_quote_reserves == 0
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u8(&mut self) -> u8 {
        let v = self.data[self.pos];
        self.pos += 1;
        v
    }

    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        v
    }

    fn u64(&mut self) -> u64 {
        let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        v
    }

    fn bool(&mut self) -> bool {
        let v = self.data[self.pos] != 0;
        self.pos += 1;
        v
    }

    fn i128(&mut self) -> i128 {
        let v = i128::from_le_bytes(self.data[self.pos..self.pos + 16].try_into().unwrap());
        self.pos += 16;
        v
    }

    fn pubkey(&mut self) -> Pubkey {
        let bytes: [u8; 32] = self.data[self.pos..self.pos + 32].try_into().unwrap();
        self.pos += 32;
        Pubkey::new_from_array(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteError {
    NotStandardPool,
    ZeroAmount,
    InsufficientLiquidity,
}

/// Gross base-token output for spending `quote_amount_in` — before any fee.
/// `base_reserves`/`quote_reserves` must be the pool's two token account
/// balances fetched immediately before quoting (not fields on `Pool`).
pub fn quote_buy_exact_quote_in(
    pool: &Pool,
    base_reserves: u64,
    quote_reserves: u64,
    quote_amount_in: u64,
) -> Result<u64, QuoteError> {
    if !pool.is_standard() {
        return Err(QuoteError::NotStandardPool);
    }
    if quote_amount_in == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    // Direct numerator/denominator, matching the real program's rounding
    // exactly (see module docs) — computing via an intermediate k and
    // subtracting is mathematically equivalent in real numbers but can be
    // off by 1 here due to where the integer floor lands.
    let numerator = (quote_amount_in as u128) * (base_reserves as u128);
    let denominator = (quote_reserves as u128) + (quote_amount_in as u128);
    let base_out = numerator / denominator;
    if base_out == 0 || base_out >= base_reserves as u128 {
        return Err(QuoteError::InsufficientLiquidity);
    }
    Ok(base_out as u64)
}

/// Gross quote-token output for selling `base_amount_in` — before any fee.
/// Verified against a real mainnet sell to be exact (not just within
/// rounding) once truncated to an integer lamport amount.
pub fn quote_sell(
    pool: &Pool,
    base_reserves: u64,
    quote_reserves: u64,
    base_amount_in: u64,
) -> Result<u64, QuoteError> {
    if !pool.is_standard() {
        return Err(QuoteError::NotStandardPool);
    }
    if base_amount_in == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    let numerator = (base_amount_in as u128) * (quote_reserves as u128);
    let denominator = (base_reserves as u128) + (base_amount_in as u128);
    Ok((numerator / denominator) as u64)
}
