//! Pump bonding curve: account decoding and constant-product quote math.
//!
//! Program ID and the `BondingCurve` account layout come from the official
//! IDL (`idl/pump.json`, vendored from `pump-fun/pump-public-docs`) and were
//! cross-checked byte-for-byte against a real mainnet account (its `space`
//! matches this struct's encoded size exactly) and against a real `sell`
//! trade's `TradeEvent` (back-solving the constant-product invariant against
//! the event's before/after reserves reproduced the real output exactly) —
//! see `tests/bonding_curve_test.rs` for the recorded fixture.
//!
//! Scope: only curves in their default configuration are priced here —
//! `is_mayhem_mode = false`, `is_cashback_coin = false`, and `quote_mint`
//! equal to the native-SOL sentinel (the System Program id; verified on a
//! real fresh curve, not the wrapped-SOL mint). Real trade data shows fees
//! are resolved through a separate, undocumented on-chain program (its
//! actual charged amount did not match its own quoted bps in the one trade
//! inspected), so this module deliberately does not attempt to compute fees
//! at all: `quote_buy`/`quote_sell` return the gross, fee-free swap amount.
//! Replay must read the real `fee`/`creator_fee` off the historical event
//! instead of recomputing it.

use solana_pubkey::Pubkey;

pub mod adapter;
pub mod events;

pub const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

/// Sentinel written into `BondingCurve.quote_mint` for a plain SOL-quoted
/// curve created via the legacy `create` instruction.
pub const NATIVE_SOL_QUOTE_SENTINEL: Pubkey = Pubkey::new_from_array([0u8; 32]);

const BONDING_CURVE_DISCRIMINATOR: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];
const BONDING_CURVE_LEN: usize = 8 + 8 * 5 + 1 + 32 + 1 + 1 + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    DiscriminatorMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BondingCurve {
    pub virtual_token_reserves: u64,
    pub virtual_quote_reserves: u64,
    pub real_token_reserves: u64,
    pub real_quote_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub quote_mint: Pubkey,
}

impl BondingCurve {
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        if data.len() < BONDING_CURVE_LEN {
            return Err(DecodeError::TooShort);
        }
        if data[..8] != BONDING_CURVE_DISCRIMINATOR {
            return Err(DecodeError::DiscriminatorMismatch);
        }
        let mut r = Reader::new(&data[8..]);
        Ok(BondingCurve {
            virtual_token_reserves: r.u64(),
            virtual_quote_reserves: r.u64(),
            real_token_reserves: r.u64(),
            real_quote_reserves: r.u64(),
            token_total_supply: r.u64(),
            complete: r.bool(),
            creator: r.pubkey(),
            is_mayhem_mode: r.bool(),
            is_cashback_coin: r.bool(),
            quote_mint: r.pubkey(),
        })
    }

    /// Curve variants this Stage 1 math is verified for. Anything else (a
    /// boosted/mayhem curve, cashback-enabled, or a non-SOL quote mint)
    /// uses pricing rules not yet reverse-engineered from real data, so
    /// callers must not price it — `quote_buy`/`quote_sell` refuse instead
    /// of guessing.
    pub fn is_standard(&self) -> bool {
        !self.is_mayhem_mode && !self.is_cashback_coin && self.quote_mint == NATIVE_SOL_QUOTE_SENTINEL
    }

    pub fn is_graduated(&self) -> bool {
        self.complete
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

    fn pubkey(&mut self) -> Pubkey {
        let bytes: [u8; 32] = self.data[self.pos..self.pos + 32].try_into().unwrap();
        self.pos += 32;
        Pubkey::new_from_array(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteError {
    NotStandardCurve,
    AlreadyGraduated,
    InsufficientLiquidity,
    ZeroAmount,
}

fn require_priceable(curve: &BondingCurve) -> Result<(), QuoteError> {
    if !curve.is_standard() {
        return Err(QuoteError::NotStandardCurve);
    }
    if curve.is_graduated() {
        return Err(QuoteError::AlreadyGraduated);
    }
    Ok(())
}

/// Gross token output for spending `quote_amount_in` lamports (matches the
/// `buy_exact_sol_in` / `buy_exact_quote_in_v2` instructions) — before any
/// protocol/creator fee.
pub fn quote_buy_exact_quote_in(curve: &BondingCurve, quote_amount_in: u64) -> Result<u64, QuoteError> {
    require_priceable(curve)?;
    if quote_amount_in == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    // Direct numerator/denominator rather than an intermediate k, to match
    // the real program's rounding exactly — see the PumpSwap sibling crate
    // for the real-trade discrepancy this avoids (same underlying pitfall
    // applies here even though the tolerance below never proved it).
    let numerator = (quote_amount_in as u128) * (curve.virtual_token_reserves as u128);
    let denominator = (curve.virtual_quote_reserves as u128) + (quote_amount_in as u128);
    let token_out = numerator / denominator;
    if token_out == 0 || token_out > curve.real_token_reserves as u128 {
        return Err(QuoteError::InsufficientLiquidity);
    }
    Ok(token_out as u64)
}

/// Gross quote cost for buying exactly `token_amount_out` tokens (matches
/// the primary `buy` instruction, which takes a desired token `amount` and
/// a `max_sol_cost` slippage cap) — before any protocol/creator fee.
pub fn quote_buy_exact_token_out(curve: &BondingCurve, token_amount_out: u64) -> Result<u64, QuoteError> {
    require_priceable(curve)?;
    if token_amount_out == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    if token_amount_out as u128 >= curve.virtual_token_reserves as u128
        || token_amount_out > curve.real_token_reserves
    {
        return Err(QuoteError::InsufficientLiquidity);
    }
    let numerator = (token_amount_out as u128) * (curve.virtual_quote_reserves as u128);
    let denominator = (curve.virtual_token_reserves as u128) - (token_amount_out as u128);
    Ok(numerator.div_ceil(denominator) as u64)
}

/// Gross quote output for selling `token_amount_in` tokens (matches the
/// `sell` instruction) — before any protocol/creator fee. Verified against
/// a real mainnet sell exactly (see tests/bonding_curve_test.rs).
pub fn quote_sell(curve: &BondingCurve, token_amount_in: u64) -> Result<u64, QuoteError> {
    require_priceable(curve)?;
    if token_amount_in == 0 {
        return Err(QuoteError::ZeroAmount);
    }
    let numerator = (token_amount_in as u128) * (curve.virtual_quote_reserves as u128);
    let denominator = (curve.virtual_token_reserves as u128) + (token_amount_in as u128);
    Ok((numerator / denominator) as u64)
}
