//! `VenueAdapter` implementation for PumpSwap. Ties together `events`
//! (decode) and `Pool`/quote math (apply_update, quote_buy, quote_sell)
//! from the rest of this crate.
//!
//! Unlike Pump, one PumpSwap "instrument" (a pool) needs *three* pieces of
//! on-chain state before it is priceable: the `Pool` account itself, and
//! the two SPL token accounts holding its real base/quote reserves (`lib.rs`
//! deliberately never fetches those — callers must). `apply_update` handles
//! all three through one uniform `AccountUpdate { pubkey, data }`, the same
//! shape Pump uses: a `Pool` account seeds a reverse index from its two
//! token account addresses back to (pool, side), so a later update to
//! either of those addresses is recognized as "this pool's base/quote
//! reserve changed" without the caller having to say which is which.
//!
//! `decode` deliberately returns a `Candidate`, not a `core::domain::Event`
//! — same reasoning as Pump's adapter: correlating a trade with Token-2022
//! extension flags on the underlying mint is an ingestion-layer concern,
//! not this adapter's.

use std::collections::HashMap;
use std::convert::Infallible;

use momentum_core::adapter_contract::{LiquidityRisk, VenueAdapter};
use solana_pubkey::Pubkey;

use crate::events::{decode_event, PumpSwapEvent};
use crate::{quote_buy_exact_quote_in, quote_sell, Pool, QuoteError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    PoolCreated {
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        creator: Pubkey,
        coin_creator: Pubkey,
        lp_mint: Pubkey,
        is_mayhem_mode: bool,
        base_amount_in: u64,
        quote_amount_in: u64,
    },
    Trade {
        pool: Pubkey,
        user: Pubkey,
        is_buy: bool,
        base_amount: u64,
        quote_amount: u64,
        lp_fee: u64,
        protocol_fee: u64,
        coin_creator: Pubkey,
        coin_creator_fee: u64,
        can_boost: bool,
    },
    Withdraw {
        pool: Pubkey,
        user: Pubkey,
        lp_token_amount_in: u64,
        base_amount_out: u64,
        quote_amount_out: u64,
        /// The LP mint's total supply *before* this withdrawal — see
        /// `events::WithdrawEvent` doc comment. PumpSwap permanently locks
        /// a small fixed amount of LP supply at pool creation (see
        /// `momentum_ingest::pumpswap::PERMANENTLY_LOCKED_MINIMUM_LIQUIDITY`),
        /// so callers determining "all liquidity removed" must compare
        /// against that floor, not a bare `lp_token_amount_in >=
        /// lp_mint_supply`.
        lp_mint_supply: u64,
    },
    Deposit {
        pool: Pubkey,
        user: Pubkey,
        base_amount_in: u64,
        quote_amount_in: u64,
    },
}

#[derive(Debug, Clone)]
pub struct AccountUpdate {
    pub pubkey: Pubkey,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub amount_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeRequest {
    pub pool: Pubkey,
    pub amount: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterError {
    UnknownInstrument,
    /// The `Pool` account has been seen, but one or both of its token
    /// accounts' balances have not — distinct from `UnknownInstrument` so
    /// callers can tell "never heard of this pool" apart from "still
    /// waiting on the rest of its state".
    ReservesNotYetKnown,
    Quote(QuoteError),
    /// Stage 1 is paper-only: decode and quote, never build or send a real
    /// transaction. Real execution is Stage 7.
    NotImplementedInStage1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Base,
    Quote,
}

#[derive(Debug, Clone, Copy, Default)]
struct PoolReserves {
    base: Option<u64>,
    quote: Option<u64>,
}

impl PoolReserves {
    fn both(&self) -> Option<(u64, u64)> {
        Some((self.base?, self.quote?))
    }
}

/// SPL Token and Token-2022 accounts share the same fixed-offset base
/// layout (`mint: Pubkey`, `owner: Pubkey`, then `amount: u64`) — Token-2022
/// only appends variable-length extension TLV data after it. Confirmed
/// against a real mainnet pool's base (legacy SPL) and quote (Token-2022)
/// reserve accounts, both matching `getTokenAccountBalance` exactly at this
/// offset.
const SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;

fn read_spl_token_amount(data: &[u8]) -> Option<u64> {
    let end = SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET + 8;
    if data.len() < end {
        return None;
    }
    Some(u64::from_le_bytes(data[SPL_TOKEN_ACCOUNT_AMOUNT_OFFSET..end].try_into().unwrap()))
}

/// Tracks the latest known `Pool` state and real token-account reserves for
/// every PumpSwap pool this process has seen account updates for, keyed by
/// the pool's own address — what every `BuyEvent`/`SellEvent`/`AccountUpdate`
/// naturally carries, so no PDA derivation is needed (unlike Pump's
/// bonding-curve-from-mint case).
#[derive(Debug, Default)]
pub struct PumpSwapAdapter {
    version: &'static str,
    pools: HashMap<Pubkey, Pool>,
    token_account_owner: HashMap<Pubkey, (Pubkey, Side)>,
    reserves: HashMap<Pubkey, PoolReserves>,
}

impl PumpSwapAdapter {
    pub fn new(version: &'static str) -> Self {
        Self { version, pools: HashMap::new(), token_account_owner: HashMap::new(), reserves: HashMap::new() }
    }

    pub fn pool(&self, pool: &Pubkey) -> Option<&Pool> {
        self.pools.get(pool)
    }

    pub fn known_reserves(&self, pool: &Pubkey) -> Option<(u64, u64)> {
        self.reserves.get(pool).and_then(PoolReserves::both)
    }
}

impl VenueAdapter for PumpSwapAdapter {
    type Instrument = Pubkey;
    type Candidate = Candidate;
    type RawEvent = [u8];
    type AccountUpdate = AccountUpdate;
    type Quote = Quote;
    type TradeRequest = TradeRequest;
    type BuiltTransaction = Infallible;
    type Error = AdapterError;

    fn decode(&self, event: &[u8]) -> Option<Candidate> {
        match decode_event(event)? {
            PumpSwapEvent::CreatePool(c) => Some(Candidate::PoolCreated {
                pool: c.pool,
                base_mint: c.base_mint,
                quote_mint: c.quote_mint,
                creator: c.creator,
                coin_creator: c.coin_creator,
                lp_mint: c.lp_mint,
                is_mayhem_mode: c.is_mayhem_mode,
                base_amount_in: c.base_amount_in,
                quote_amount_in: c.quote_amount_in,
            }),
            PumpSwapEvent::Buy(b) => Some(Candidate::Trade {
                pool: b.pool,
                user: b.user,
                is_buy: true,
                base_amount: b.base_amount_out,
                quote_amount: b.user_quote_amount_in,
                lp_fee: b.lp_fee,
                protocol_fee: b.protocol_fee,
                coin_creator: b.coin_creator,
                coin_creator_fee: b.coin_creator_fee,
                can_boost: b.can_boost,
            }),
            PumpSwapEvent::Sell(s) => Some(Candidate::Trade {
                pool: s.pool,
                user: s.user,
                is_buy: false,
                base_amount: s.base_amount_in,
                quote_amount: s.user_quote_amount_out,
                lp_fee: s.lp_fee,
                protocol_fee: s.protocol_fee,
                coin_creator: s.coin_creator,
                coin_creator_fee: s.coin_creator_fee,
                can_boost: s.can_boost,
            }),
            PumpSwapEvent::Withdraw(w) => Some(Candidate::Withdraw {
                pool: w.pool,
                user: w.user,
                lp_token_amount_in: w.lp_token_amount_in,
                base_amount_out: w.base_amount_out,
                quote_amount_out: w.quote_amount_out,
                lp_mint_supply: w.lp_mint_supply,
            }),
            PumpSwapEvent::Deposit(d) => {
                Some(Candidate::Deposit { pool: d.pool, user: d.user, base_amount_in: d.base_amount_in, quote_amount_in: d.quote_amount_in })
            }
        }
    }

    fn apply_update(&mut self, update: &AccountUpdate) -> Result<(), AdapterError> {
        if let Ok(pool) = Pool::decode(&update.data) {
            self.token_account_owner.insert(pool.pool_base_token_account, (update.pubkey, Side::Base));
            self.token_account_owner.insert(pool.pool_quote_token_account, (update.pubkey, Side::Quote));
            self.pools.insert(update.pubkey, pool);
            return Ok(());
        }
        // Not every account this adapter is fed an update for is a Pool —
        // it might be one of that pool's two reserve token accounts
        // instead, recognized only once we've decoded the owning Pool.
        // Anything else (an unrelated account) is expected and not an
        // error, just nothing to cache here.
        if let Some(&(pool_key, side)) = self.token_account_owner.get(&update.pubkey) {
            if let Some(amount) = read_spl_token_amount(&update.data) {
                let entry = self.reserves.entry(pool_key).or_default();
                match side {
                    Side::Base => entry.base = Some(amount),
                    Side::Quote => entry.quote = Some(amount),
                }
            }
        }
        Ok(())
    }

    fn quote_buy(&self, instrument: &Pubkey, amount_in: u64) -> Result<Quote, AdapterError> {
        let pool = self.pools.get(instrument).ok_or(AdapterError::UnknownInstrument)?;
        let (base_reserves, quote_reserves) =
            self.reserves.get(instrument).and_then(PoolReserves::both).ok_or(AdapterError::ReservesNotYetKnown)?;
        let amount_out =
            quote_buy_exact_quote_in(pool, base_reserves, quote_reserves, amount_in).map_err(AdapterError::Quote)?;
        Ok(Quote { amount_out })
    }

    fn quote_sell(&self, instrument: &Pubkey, amount_in: u64) -> Result<Quote, AdapterError> {
        let pool = self.pools.get(instrument).ok_or(AdapterError::UnknownInstrument)?;
        let (base_reserves, quote_reserves) =
            self.reserves.get(instrument).and_then(PoolReserves::both).ok_or(AdapterError::ReservesNotYetKnown)?;
        let amount_out = quote_sell(pool, base_reserves, quote_reserves, amount_in).map_err(AdapterError::Quote)?;
        Ok(Quote { amount_out })
    }

    fn build_buy(&self, _request: &TradeRequest) -> Result<Infallible, AdapterError> {
        Err(AdapterError::NotImplementedInStage1)
    }

    fn build_sell(&self, _request: &TradeRequest) -> Result<Infallible, AdapterError> {
        Err(AdapterError::NotImplementedInStage1)
    }

    fn liquidity_risk(&self, instrument: &Pubkey) -> LiquidityRisk {
        match self.pools.get(instrument) {
            None => LiquidityRisk::Unpriceable,
            Some(pool) if !pool.is_standard() => LiquidityRisk::Unpriceable,
            Some(_) => match self.reserves.get(instrument).and_then(PoolReserves::both) {
                Some(_) => LiquidityRisk::Healthy,
                None => LiquidityRisk::Unpriceable,
            },
        }
    }

    fn protocol_version(&self) -> &'static str {
        self.version
    }
}
