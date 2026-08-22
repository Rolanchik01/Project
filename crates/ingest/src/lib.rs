//! Merges one venue's raw `Candidate` with correlated on-chain state (e.g. a
//! Token-2022 mint inspection, a pool's cached reserves) into
//! `core::domain::Event` — the shape `risk_engine`/`replay` already
//! consume. Deliberately its own crate: `core` cannot depend on
//! `pump`/`pumpswap` (they depend on `core` for the `VenueAdapter` trait),
//! so the glue that knows about both sides has to sit above all of them.
//!
//! - `pump` — Pump's three candidates: `TokenCreated` (the anti-rug-critical
//!   path), `Trade` -> `Buy`/`Sell`, `Graduated` -> `Graduation`.
//! - `pumpswap` — PumpSwap's `Trade` -> `Buy`/`Sell` and `PoolCreated`.
//! - `price` — the SOL/USD lamport conversion both venues' trade/liquidity
//!   ingestion share. It does not fetch a price; callers supply
//!   `sol_usd_price` from wherever the live ingestion driver gets one — no
//!   price source has been chosen or verified yet, so fetching one isn't
//!   built here rather than invented.
//!
//! One `Candidate::TokenCreated` produces exactly **one** `Event`, not two.
//! An earlier draft of this module also emitted a `CurveCreated` marker
//! event alongside it, reasoning that Pump's `create` instruction brings
//! both the mint and the bonding curve into existence atomically. That
//! turned out to be wrong: `core::dedup`'s dedupe key is
//! `(venue, signature, instruction_index)` — one raw on-chain instruction
//! is assumed to produce exactly one `Event`, and every existing Stage 0
//! fixture (`crates/core/tests/support/mod.rs`) follows that rule. Two
//! events sharing one instruction's key would silently collide in
//! `dedupe_events` (a `HashMap` keyed by that tuple keeps only one) and in
//! `StreamDeduplicator` (the second would be rejected as a "duplicate").
//! `CurveCreated` also isn't read by `risk_engine::hard_blocks` or
//! `snapshot` today, so nothing downstream needs it yet — only
//! `TokenCreated` carries the safety flags the hard-veto gate depends on.
//!
//! Also not mapped into `TokenCreated`: `Candidate::TokenCreated`'s
//! `is_mayhem_mode`, `is_cashback_enabled`, and non-standard `quote_mint`
//! fields. Those are *venue pricing* risk (`momentum_pump::BondingCurve::
//! is_standard`, surfaced through `VenueAdapter::liquidity_risk`), not
//! *token* safety — folding them into `TokenCreated`'s hard blocks would
//! conflate "this curve can't be priced safely" with "this mint can rug
//! holders", which are different failure modes needing different handling
//! downstream. A trading decision must still gate on both `risk_engine`'s
//! verdict *and* `liquidity_risk() == Healthy` before ever sizing a
//! position — that combination isn't wired up yet (no decision/execution
//! glue exists before this crate).

pub mod price;
pub mod pump;
pub mod pumpswap;

pub use pump::{ingest_pump_graduated, ingest_pump_token_created, ingest_pump_trade};
pub use pumpswap::{ingest_pumpswap_pool_created, ingest_pumpswap_trade};

use solana_pubkey::Pubkey;

/// The legacy SPL Token program — a mint owned by this program has no
/// Token-2022 extensions at all (nothing for `inspect_mint` to find), which
/// is fine and expected, not itself a red flag.
pub const LEGACY_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

/// Everything about a raw event's place in the chain that a `Candidate`
/// itself doesn't carry (it only holds one instruction's decoded payload) —
/// supplied by whatever fed the adapter the raw bytes in the first place
/// (the recorder / RPC ingestion driver knows the slot, signature, and
/// instruction index the log came from).
#[derive(Debug, Clone)]
pub struct EventContext {
    pub id: String,
    pub slot: u64,
    pub observed_at_ns: u64,
    pub signature: String,
    pub instruction_index: u32,
    pub program_version: String,
}

pub(crate) fn is_known_token_program(token_program: &Pubkey) -> bool {
    let legacy: Pubkey = LEGACY_TOKEN_PROGRAM_ID.parse().expect("LEGACY_TOKEN_PROGRAM_ID is a valid pubkey");
    let token_2022: Pubkey = TOKEN_2022_PROGRAM_ID.parse().expect("TOKEN_2022_PROGRAM_ID is a valid pubkey");
    *token_program == legacy || *token_program == token_2022
}
