//! `VenueAdapter` implementation for Pump. Ties together `events` (decode)
//! and `BondingCurve`/quote math (apply_update, quote_buy, quote_sell) from
//! the rest of this crate.
//!
//! `decode` deliberately returns a `Candidate`, not a `core::domain::Event`:
//! `Candidate::TokenCreated` carries everything a `CreateEvent` gives us
//! (creator, mint, initial reserves, mayhem/cashback flags) but *not*
//! Token-2022 extension flags (transfer fee/hook, permanent delegate, ...)
//! — those live on the mint account, a separate piece of on-chain state
//! this decode call was never given. Merging a `Candidate` with a
//! `momentum_token2022::inspect_mint` result into a scoreable
//! `core::domain::Event` is the ingestion layer's job, not this adapter's.

use std::collections::HashMap;
use std::convert::Infallible;

use momentum_core::adapter_contract::{LiquidityRisk, VenueAdapter};
use solana_pubkey::Pubkey;

use crate::events::{decode_event, PumpEvent};
use crate::{quote_buy_exact_quote_in, quote_sell, BondingCurve, QuoteError, PUMP_PROGRAM_ID};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    TokenCreated {
        mint: Pubkey,
        bonding_curve: Pubkey,
        creator: Pubkey,
        user: Pubkey,
        name: String,
        symbol: String,
        uri: String,
        token_program: Pubkey,
        is_mayhem_mode: bool,
        is_cashback_enabled: bool,
        quote_mint: Pubkey,
    },
    Trade {
        mint: Pubkey,
        bonding_curve: Pubkey,
        user: Pubkey,
        is_buy: bool,
        quote_amount: u64,
        token_amount: u64,
        fee: u64,
        creator_fee: u64,
        creator: Pubkey,
    },
    Graduated {
        mint: Pubkey,
        bonding_curve: Pubkey,
        user: Pubkey,
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
    pub bonding_curve: Pubkey,
    pub amount: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterError {
    UnknownInstrument,
    Quote(QuoteError),
    /// Stage 1 is paper-only: decode and quote, never build or send a real
    /// transaction. Real execution is Stage 7.
    NotImplementedInStage1,
}

/// Tracks the latest known state of every bonding curve this process has
/// seen an account update for, keyed by the curve's own address (what
/// `apply_update` is naturally given — a raw account is identified by its
/// own pubkey, not by the mint it prices). Use `bonding_curve_pda` to get
/// from a mint to that key when you only have the mint (e.g. from a
/// `Candidate::Trade`, which doesn't carry the curve address directly).
#[derive(Debug, Default)]
pub struct PumpAdapter {
    version: &'static str,
    curves: HashMap<Pubkey, BondingCurve>,
}

impl PumpAdapter {
    pub fn new(version: &'static str) -> Self {
        Self { version, curves: HashMap::new() }
    }

    /// Derives the bonding-curve PDA for a mint (seeds = ["bonding-curve",
    /// mint], program = Pump) — a pure computation, no RPC round trip.
    pub fn bonding_curve_pda(mint: &Pubkey) -> Pubkey {
        let program: Pubkey = PUMP_PROGRAM_ID.parse().expect("PUMP_PROGRAM_ID is a valid pubkey");
        Pubkey::find_program_address(&[b"bonding-curve", mint.as_ref()], &program).0
    }

    pub fn curve(&self, bonding_curve: &Pubkey) -> Option<&BondingCurve> {
        self.curves.get(bonding_curve)
    }
}

impl VenueAdapter for PumpAdapter {
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
            PumpEvent::Create(c) => Some(Candidate::TokenCreated {
                mint: c.mint,
                bonding_curve: c.bonding_curve,
                creator: c.creator,
                user: c.user,
                name: c.name,
                symbol: c.symbol,
                uri: c.uri,
                token_program: c.token_program,
                is_mayhem_mode: c.is_mayhem_mode,
                is_cashback_enabled: c.is_cashback_enabled,
                quote_mint: c.quote_mint,
            }),
            PumpEvent::Trade(t) => Some(Candidate::Trade {
                mint: t.mint,
                bonding_curve: Self::bonding_curve_pda(&t.mint),
                user: t.user,
                is_buy: t.is_buy,
                quote_amount: t.quote_amount,
                token_amount: t.token_amount,
                fee: t.fee,
                creator_fee: t.creator_fee,
                creator: t.creator,
            }),
            PumpEvent::Complete(c) => Some(Candidate::Graduated { mint: c.mint, bonding_curve: c.bonding_curve, user: c.user }),
        }
    }

    fn apply_update(&mut self, update: &AccountUpdate) -> Result<(), AdapterError> {
        // Not every account this adapter is fed an update for is a
        // BondingCurve (a real pipeline also delivers the mint account, for
        // Token-2022 inspection elsewhere) — failing to decode as one is
        // expected and not an error, just nothing to cache here.
        if let Ok(curve) = BondingCurve::decode(&update.data) {
            self.curves.insert(update.pubkey, curve);
        }
        Ok(())
    }

    fn quote_buy(&self, instrument: &Pubkey, amount_in: u64) -> Result<Quote, AdapterError> {
        let curve = self.curves.get(instrument).ok_or(AdapterError::UnknownInstrument)?;
        let amount_out = quote_buy_exact_quote_in(curve, amount_in).map_err(AdapterError::Quote)?;
        Ok(Quote { amount_out })
    }

    fn quote_sell(&self, instrument: &Pubkey, amount_in: u64) -> Result<Quote, AdapterError> {
        let curve = self.curves.get(instrument).ok_or(AdapterError::UnknownInstrument)?;
        let amount_out = quote_sell(curve, amount_in).map_err(AdapterError::Quote)?;
        Ok(Quote { amount_out })
    }

    fn build_buy(&self, _request: &TradeRequest) -> Result<Infallible, AdapterError> {
        Err(AdapterError::NotImplementedInStage1)
    }

    fn build_sell(&self, _request: &TradeRequest) -> Result<Infallible, AdapterError> {
        Err(AdapterError::NotImplementedInStage1)
    }

    fn liquidity_risk(&self, instrument: &Pubkey) -> LiquidityRisk {
        match self.curves.get(instrument) {
            None => LiquidityRisk::Unpriceable,
            Some(curve) if curve.is_graduated() => LiquidityRisk::Graduated,
            Some(curve) if !curve.is_standard() => LiquidityRisk::Unpriceable,
            Some(_) => LiquidityRisk::Healthy,
        }
    }

    fn protocol_version(&self) -> &'static str {
        self.version
    }
}
