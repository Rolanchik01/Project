//! PumpSwap's Anchor self-CPI event log decoding: `CreatePoolEvent`,
//! `BuyEvent`, `SellEvent`, `WithdrawEvent`, `DepositEvent`. Field order
//! and discriminators come from the vendored IDL (`idl/pump_amm.json`).
//!
//! `BuyEvent` and `SellEvent` are verified byte-for-byte against real
//! mainnet transactions on non-boosted pools — see `tests/events_test.rs`.
//! That research also confirmed which event field is the AMM's raw
//! constant-product output: `SellEvent.quote_amount_out` and (for a
//! `buy_exact_quote_in` trade) `BuyEvent.base_amount_out` both matched
//! `lib.rs`'s `quote_sell`/`quote_buy_exact_quote_in` exactly when fed the
//! event's own pre-trade `pool_*_token_reserves` — the plain `buy`
//! instruction (fixed token amount out, variable SOL in) is priced the
//! other way around and does *not* match that formula, as expected.
//!
//! `CreatePoolEvent` uses the same decode methodology, applied
//! consistently, and is now also verified byte-for-byte against a real one
//! — see `tests/events_test.rs`. A historical scan of ~250+ recent
//! PumpSwap signatures had found none; it was only confirmed by
//! subscribing live and waiting for one to happen in real time, same as
//! Pump's `CompleteEvent`.

use solana_pubkey::Pubkey;

const CREATE_POOL_EVENT_DISCRIMINATOR: [u8; 8] = [177, 49, 12, 210, 160, 118, 167, 116];
const BUY_EVENT_DISCRIMINATOR: [u8; 8] = [103, 244, 82, 31, 44, 245, 119, 119];
const SELL_EVENT_DISCRIMINATOR: [u8; 8] = [62, 47, 55, 10, 165, 3, 220, 42];
const WITHDRAW_EVENT_DISCRIMINATOR: [u8; 8] = [22, 9, 133, 26, 160, 44, 71, 192];
const DEPOSIT_EVENT_DISCRIMINATOR: [u8; 8] = [120, 248, 61, 83, 31, 142, 107, 144];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePoolEvent {
    pub pool: Pubkey,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub coin_creator: Pubkey,
    pub base_amount_in: u64,
    pub quote_amount_in: u64,
    pub is_mayhem_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyEvent {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub base_amount_out: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub user_quote_amount_in: u64,
    pub lp_fee: u64,
    pub protocol_fee: u64,
    pub coin_creator: Pubkey,
    pub coin_creator_fee: u64,
    pub ix_name: String,
    pub virtual_quote_reserves: i128,
    pub can_boost: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellEvent {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub base_amount_in: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub user_quote_amount_out: u64,
    pub lp_fee: u64,
    pub protocol_fee: u64,
    pub coin_creator: Pubkey,
    pub coin_creator_fee: u64,
    pub virtual_quote_reserves: i128,
    pub can_boost: bool,
}

/// A liquidity withdrawal (LP token burn -> base+quote out). Field order
/// verified byte-for-byte against a real mainnet withdrawal — see
/// `tests/events_test.rs` — and, unlike `BuyEvent`/`SellEvent`, matches the
/// vendored IDL's declared field list and order exactly (every byte
/// consumed, nothing left over).
///
/// `pool_base_token_reserves`/`pool_quote_token_reserves` follow the same
/// pre-instruction convention confirmed for `BuyEvent`/`SellEvent`: the
/// real withdrawal observed here had `base_amount_out` almost exactly
/// equal to `pool_base_token_reserves` (off by single-digit dust), meaning
/// those reserves are the pool's state *before* this withdrawal drained
/// nearly all of it — not after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawEvent {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub lp_token_amount_in: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub base_amount_out: u64,
    pub quote_amount_out: u64,
    /// The LP mint's total supply *before* this withdrawal burns
    /// `lp_token_amount_in` of it — confirmed against a real withdrawal
    /// that burned all but 100 raw units of this exact value, matching
    /// `base_amount_out` draining nearly all of `pool_base_token_reserves`
    /// in the same event.
    pub lp_mint_supply: u64,
}

/// A liquidity deposit (base+quote in -> LP token mint). Field order
/// verified byte-for-byte against a real mainnet deposit — see
/// `tests/events_test.rs` — and matches the vendored IDL's declared field
/// list and order exactly, same as `WithdrawEvent`.
///
/// A second, much shorter (105-byte, vs. this layout's 248) raw event was
/// also observed live under the same discriminator, bundled in the same
/// transaction as a `SellEvent` — almost certainly a genuine alternate
/// `DepositEvent` shape from a "sell then deposit" code path with some
/// fields serialized as absent (Borsh `Option<T>::None`) rather than a
/// different event type entirely (a stray byte between two of its pubkeys
/// decoded exactly as a Borsh `Some` tag). `decode_deposit` only handles
/// the full shape below and returns `None` for that shorter one — refusing
/// to guess at a layout not yet confirmed, the same fail-closed stance
/// this crate already takes for boosted/mayhem pools it can't price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositEvent {
    pub pool: Pubkey,
    pub user: Pubkey,
    pub lp_token_amount_out: u64,
    pub pool_base_token_reserves: u64,
    pub pool_quote_token_reserves: u64,
    pub base_amount_in: u64,
    pub quote_amount_in: u64,
    pub lp_mint_supply: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PumpSwapEvent {
    CreatePool(CreatePoolEvent),
    Buy(BuyEvent),
    Sell(SellEvent),
    Withdraw(WithdrawEvent),
    Deposit(DepositEvent),
}

/// Decodes one Anchor self-CPI event log (8-byte discriminator + payload).
/// Returns `None` for anything that isn't one of the three event kinds this
/// bot needs (boost/cashback/admin/config events carry no trading signal
/// and are deliberately not decoded).
pub fn decode_event(data: &[u8]) -> Option<PumpSwapEvent> {
    if data.len() < 8 {
        return None;
    }
    let (disc, body) = data.split_at(8);
    match disc {
        d if d == CREATE_POOL_EVENT_DISCRIMINATOR => decode_create_pool(body).map(PumpSwapEvent::CreatePool),
        d if d == BUY_EVENT_DISCRIMINATOR => decode_buy(body).map(PumpSwapEvent::Buy),
        d if d == SELL_EVENT_DISCRIMINATOR => decode_sell(body).map(PumpSwapEvent::Sell),
        d if d == WITHDRAW_EVENT_DISCRIMINATOR => decode_withdraw(body).map(PumpSwapEvent::Withdraw),
        d if d == DEPOSIT_EVENT_DISCRIMINATOR => decode_deposit(body).map(PumpSwapEvent::Deposit),
        _ => None,
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

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn u8(&mut self) -> Option<u8> {
        if self.remaining() < 1 {
            return None;
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Some(v)
    }

    fn u16(&mut self) -> Option<u16> {
        if self.remaining() < 2 {
            return None;
        }
        let v = u16::from_le_bytes(self.data[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        Some(v)
    }

    fn u64(&mut self) -> Option<u64> {
        if self.remaining() < 8 {
            return None;
        }
        let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Some(v)
    }

    fn i64(&mut self) -> Option<i64> {
        if self.remaining() < 8 {
            return None;
        }
        let v = i64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Some(v)
    }

    fn i128(&mut self) -> Option<i128> {
        if self.remaining() < 16 {
            return None;
        }
        let v = i128::from_le_bytes(self.data[self.pos..self.pos + 16].try_into().unwrap());
        self.pos += 16;
        Some(v)
    }

    fn bool(&mut self) -> Option<bool> {
        if self.remaining() < 1 {
            return None;
        }
        let v = self.data[self.pos] != 0;
        self.pos += 1;
        Some(v)
    }

    fn pubkey(&mut self) -> Option<Pubkey> {
        if self.remaining() < 32 {
            return None;
        }
        let bytes: [u8; 32] = self.data[self.pos..self.pos + 32].try_into().unwrap();
        self.pos += 32;
        Some(Pubkey::new_from_array(bytes))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u32()?;
        if self.remaining() < len as usize {
            return None;
        }
        let s = String::from_utf8(self.data[self.pos..self.pos + len as usize].to_vec()).ok()?;
        self.pos += len as usize;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        if self.remaining() < 4 {
            return None;
        }
        let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Some(v)
    }
}

fn decode_create_pool(body: &[u8]) -> Option<CreatePoolEvent> {
    let mut r = Reader::new(body);
    let _timestamp = r.i64()?;
    let _index = r.u16()?;
    let creator = r.pubkey()?;
    let base_mint = r.pubkey()?;
    let quote_mint = r.pubkey()?;
    let _base_mint_decimals = r.u8()?;
    let _quote_mint_decimals = r.u8()?;
    let base_amount_in = r.u64()?;
    let quote_amount_in = r.u64()?;
    let _pool_base_amount = r.u64()?;
    let _pool_quote_amount = r.u64()?;
    let _minimum_liquidity = r.u64()?;
    let _initial_liquidity = r.u64()?;
    let _lp_token_amount_out = r.u64()?;
    let _pool_bump = r.u8()?;
    let pool = r.pubkey()?;
    let lp_mint = r.pubkey()?;
    let _user_base_token_account = r.pubkey()?;
    let _user_quote_token_account = r.pubkey()?;
    let coin_creator = r.pubkey()?;
    let is_mayhem_mode = r.bool()?;

    Some(CreatePoolEvent { pool, creator, base_mint, quote_mint, lp_mint, coin_creator, base_amount_in, quote_amount_in, is_mayhem_mode })
}

fn decode_buy(body: &[u8]) -> Option<BuyEvent> {
    let mut r = Reader::new(body);
    let _timestamp = r.i64()?;
    let base_amount_out = r.u64()?;
    let _max_quote_amount_in = r.u64()?;
    let _user_base_token_reserves = r.u64()?;
    let _user_quote_token_reserves = r.u64()?;
    let pool_base_token_reserves = r.u64()?;
    let pool_quote_token_reserves = r.u64()?;
    let _quote_amount_in = r.u64()?;
    let _lp_fee_basis_points = r.u64()?;
    let lp_fee = r.u64()?;
    let _protocol_fee_basis_points = r.u64()?;
    let protocol_fee = r.u64()?;
    let _quote_amount_in_with_lp_fee = r.u64()?;
    let user_quote_amount_in = r.u64()?;
    let pool = r.pubkey()?;
    let user = r.pubkey()?;
    let _user_base_token_account = r.pubkey()?;
    let _user_quote_token_account = r.pubkey()?;
    let _protocol_fee_recipient = r.pubkey()?;
    let _protocol_fee_recipient_token_account = r.pubkey()?;
    let coin_creator = r.pubkey()?;
    let _coin_creator_fee_basis_points = r.u64()?;
    let coin_creator_fee = r.u64()?;
    let _track_volume = r.bool()?;
    let _total_unclaimed_tokens = r.u64()?;
    let _total_claimed_tokens = r.u64()?;
    let _current_sol_volume = r.u64()?;
    let _last_update_timestamp = r.i64()?;
    let _min_base_amount_out = r.u64()?;
    let ix_name = r.string()?;
    let _cashback_fee_basis_points = r.u64()?;
    let _cashback = r.u64()?;
    let _buyback_fee_basis_points = r.u64()?;
    let _buyback_fee = r.u64()?;
    let virtual_quote_reserves = r.i128()?;
    let can_boost = r.bool()?;
    let _base_supply = r.u64()?;

    Some(BuyEvent {
        pool,
        user,
        base_amount_out,
        pool_base_token_reserves,
        pool_quote_token_reserves,
        user_quote_amount_in,
        lp_fee,
        protocol_fee,
        coin_creator,
        coin_creator_fee,
        ix_name,
        virtual_quote_reserves,
        can_boost,
    })
}

fn decode_sell(body: &[u8]) -> Option<SellEvent> {
    let mut r = Reader::new(body);
    let _timestamp = r.i64()?;
    let base_amount_in = r.u64()?;
    let _min_quote_amount_out = r.u64()?;
    let _user_base_token_reserves = r.u64()?;
    let _user_quote_token_reserves = r.u64()?;
    let pool_base_token_reserves = r.u64()?;
    let pool_quote_token_reserves = r.u64()?;
    let _quote_amount_out = r.u64()?;
    let _lp_fee_basis_points = r.u64()?;
    let lp_fee = r.u64()?;
    let _protocol_fee_basis_points = r.u64()?;
    let protocol_fee = r.u64()?;
    let _quote_amount_out_without_lp_fee = r.u64()?;
    let user_quote_amount_out = r.u64()?;
    let pool = r.pubkey()?;
    let user = r.pubkey()?;
    let _user_base_token_account = r.pubkey()?;
    let _user_quote_token_account = r.pubkey()?;
    let _protocol_fee_recipient = r.pubkey()?;
    let _protocol_fee_recipient_token_account = r.pubkey()?;
    let coin_creator = r.pubkey()?;
    let _coin_creator_fee_basis_points = r.u64()?;
    let coin_creator_fee = r.u64()?;
    let _cashback_fee_basis_points = r.u64()?;
    let _cashback = r.u64()?;
    let _buyback_fee_basis_points = r.u64()?;
    let _buyback_fee = r.u64()?;
    let virtual_quote_reserves = r.i128()?;
    let can_boost = r.bool()?;
    let _base_supply = r.u64()?;

    Some(SellEvent {
        pool,
        user,
        base_amount_in,
        pool_base_token_reserves,
        pool_quote_token_reserves,
        user_quote_amount_out,
        lp_fee,
        protocol_fee,
        coin_creator,
        coin_creator_fee,
        virtual_quote_reserves,
        can_boost,
    })
}

fn decode_withdraw(body: &[u8]) -> Option<WithdrawEvent> {
    let mut r = Reader::new(body);
    let _timestamp = r.i64()?;
    let lp_token_amount_in = r.u64()?;
    let _min_base_amount_out = r.u64()?;
    let _min_quote_amount_out = r.u64()?;
    let _user_base_token_reserves = r.u64()?;
    let _user_quote_token_reserves = r.u64()?;
    let pool_base_token_reserves = r.u64()?;
    let pool_quote_token_reserves = r.u64()?;
    let base_amount_out = r.u64()?;
    let quote_amount_out = r.u64()?;
    let lp_mint_supply = r.u64()?;
    let pool = r.pubkey()?;
    let user = r.pubkey()?;
    let _user_base_token_account = r.pubkey()?;
    let _user_quote_token_account = r.pubkey()?;
    let _user_pool_token_account = r.pubkey()?;

    Some(WithdrawEvent {
        pool,
        user,
        lp_token_amount_in,
        pool_base_token_reserves,
        pool_quote_token_reserves,
        base_amount_out,
        quote_amount_out,
        lp_mint_supply,
    })
}

fn decode_deposit(body: &[u8]) -> Option<DepositEvent> {
    let mut r = Reader::new(body);
    let _timestamp = r.i64()?;
    let lp_token_amount_out = r.u64()?;
    let _max_base_amount_in = r.u64()?;
    let _max_quote_amount_in = r.u64()?;
    let _user_base_token_reserves = r.u64()?;
    let _user_quote_token_reserves = r.u64()?;
    let pool_base_token_reserves = r.u64()?;
    let pool_quote_token_reserves = r.u64()?;
    let base_amount_in = r.u64()?;
    let quote_amount_in = r.u64()?;
    let lp_mint_supply = r.u64()?;
    let pool = r.pubkey()?;
    let user = r.pubkey()?;
    let _user_base_token_account = r.pubkey()?;
    let _user_quote_token_account = r.pubkey()?;
    let _user_pool_token_account = r.pubkey()?;

    Some(DepositEvent {
        pool,
        user,
        lp_token_amount_out,
        pool_base_token_reserves,
        pool_quote_token_reserves,
        base_amount_in,
        quote_amount_in,
        lp_mint_supply,
    })
}
