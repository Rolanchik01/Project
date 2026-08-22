//! Creator reputation, built entirely from what this process has itself
//! observed on-chain — not a historical crawl. Stage 1's `TokenCreated`
//! event has always carried `creator_cluster_id`/`creator_history_score`
//! fields (`crates/core/src/domain.rs`), but every ingestion path has fed
//! them `None` (see `crates/ingest/src/pump.rs`'s doc comment, which
//! called this "wallet-intelligence data" not yet built). This crate is
//! that data source for the `history_score` half.
//!
//! # Why an observed ledger, not a historical crawl
//!
//! The alternative considered was reconstructing a creator's full history
//! via `getSignaturesForAddress` the first time we see them: fetch every
//! signature, decode every transaction, find their prior `CreateEvent`s,
//! then for each prior mint independently re-derive whether it later got
//! rugged (which itself requires finding that mint's eventual PumpSwap
//! pool and its withdrawal history — the exact same mint -> pool -> rug
//! chain this ledger already needs to build for live observation). That
//! approach was rejected for three concrete reasons, not just "it's more
//! work":
//! - It solves the mint -> pool -> rug lookup *twice* (once historically,
//!   once live) instead of once.
//! - A creator wallet's signature history mixes every transaction they've
//!   ever sent, not just `Create` instructions — reliably isolating "this
//!   signature is a Pump token creation by this creator" from arbitrary
//!   history requires fetching and parsing full transactions at whatever
//!   volume that wallet produced, with no guarantee of finishing before
//!   RPC rate limits bite, for a feature whose entire purpose is a graceful
//!   fallback (`None` -> `probe_entry`, not a hard block) when that data
//!   isn't available yet.
//! - It fights the design the risk-engine already has for exactly this
//!   case: an unscored creator is meant to be treated as "unknown, so
//!   size down" (`probe_entry`), not as a blocker to work around. A ledger
//!   that starts empty and only earns confidence from what this process
//!   has verified itself is *more* aligned with that design than a
//!   best-effort historical reconstruction would be, not less.
//!
//! # What counts as a "bad" prior mint
//!
//! A mint from a creator's history counts against their score once either:
//! - Its `TokenCreated` event was hard-blocked by `risk_engine`
//!   ([`CreatorLedger::observe_hard_blocked`], fed by the live pipeline
//!   reading its own `RiskSnapshot`) — any of `risk_engine::hard_blocks`'s
//!   four checks, not just the Token-2022 extension one: an active mint or
//!   freeze authority and an unsupported token program hard-block a fresh
//!   `TokenCreated` too, and all four are checkable at creation time, or
//! - It graduated to a PumpSwap pool that later saw a real `WithdrawEvent`
//!   draining the pool down to (or below) PumpSwap's permanently-locked
//!   minimum liquidity — the same `all_liquidity_removed` flag
//!   `momentum_ingest::pumpswap::ingest_pumpswap_withdraw` already computes
//!   ([`CreatorLedger::observe_liquidity_removed`]).
//!
//! Neither check requires knowing *who* executed the withdrawal — a real
//! near-total drain shortly after a creator's own launch is bad signal for
//! that creator's track record whether or not their own wallet signed the
//! withdraw transaction (linking a withdrawer to the original creator is
//! wallet clustering, a separate, harder problem this module doesn't
//! attempt). A mint that hasn't graduated yet, or has graduated but never
//! seen a full drain, is *not* held against its creator — no evidence of
//! wrongdoing is treated as no penalty, not as suspicion.
//!
//! # Verification note
//!
//! Unlike this project's protocol decoders, there is no "real" ground
//! truth to check this module's scoring formula against — it is our own
//! bookkeeping policy, not a wire format someone else defined. The tests
//! here verify the policy behaves exactly as designed (a first-ever
//! creator is `None`, a bad mint lowers a later score, a clean history
//! keeps it at `1.0`, persistence round-trips exactly), not that it
//! matches some external reference.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One raw, replayable fact this ledger has learned. Kept deliberately
/// separate from `core::domain::Event` — `EventPayload::TokenCreated` never
/// carried the raw `creator` pubkey (only the already-computed
/// `creator_history_score`), so replaying the domain-event log alone can't
/// rebuild this ledger. These facts are this crate's own persistence,
/// written and replayed independently of `NdjsonRecorder`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
enum LedgerFact {
    Creation { creator: String, mint: String },
    PoolLinked { mint: String, pool_id: String },
    Rugged { pool_id: String },
    HardBlocked { mint: String },
}

/// Tracks which mints each creator has launched and which of those mints
/// turned out badly, purely from facts this process observed itself (see
/// module doc comment for why). Cheap to keep entirely in memory — a
/// creator with thousands of mints is not a real-world case this bot needs
/// to handle efficiently at scale in Stage 1.
#[derive(Debug, Default)]
pub struct CreatorLedger {
    /// A `HashSet`, not a `Vec`: a duplicate `Creation{creator, mint}` fact
    /// (a corrupted/duplicated persisted line, in principle) must not
    /// silently inflate a creator's mint count and skew `history_score` —
    /// the same idempotence `wallet_mints`/`wallet_has_sold` already have
    /// in `wallet::TraderLedger`, applied here too.
    creator_mints: HashMap<String, HashSet<String>>,
    mint_pool: HashMap<String, String>,
    rugged_pools: HashSet<String>,
    flagged_mints: HashSet<String>,
    persist_path: Option<PathBuf>,
}

impl CreatorLedger {
    /// A ledger that keeps no persistence file — every fact lives only for
    /// this process's lifetime. Useful for tests and for callers that
    /// manage their own persistence.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Loads a ledger from `path`'s previously-persisted facts (if the file
    /// exists — a fresh deployment with no history yet is not an error),
    /// then keeps appending new facts to the same file as they're observed.
    /// A line that fails to parse is skipped with a warning to stderr
    /// rather than aborting startup — one corrupted line (e.g. a truncated
    /// write from a prior crash) losing that one fact is far better than
    /// the whole reputation history refusing to load.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut ledger = Self { persist_path: Some(path.clone()), ..Self::default() };

        match fs::File::open(&path) {
            Ok(file) => {
                for (line_no, line) in io::BufReader::new(file).lines().enumerate() {
                    // A line that fails even to read (e.g. invalid UTF-8
                    // from a torn write) is skipped the same as one that
                    // reads fine but fails to parse as JSON below — this
                    // loop's whole point is that one bad line must not
                    // fail `load()` and abort startup via the caller's
                    // `.expect()`, and `?` on `line` would do exactly that.
                    let line = match line {
                        Ok(line) => line,
                        Err(e) => {
                            eprintln!("creator ledger: skipping unreadable line at {}:{}: {e}", path.display(), line_no + 1);
                            continue;
                        }
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<LedgerFact>(&line) {
                        Ok(fact) => ledger.apply(&fact),
                        Err(e) => eprintln!("creator ledger: skipping unparseable fact at {}:{}: {e}", path.display(), line_no + 1),
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        Ok(ledger)
    }

    fn apply(&mut self, fact: &LedgerFact) {
        match fact {
            LedgerFact::Creation { creator, mint } => {
                self.creator_mints.entry(creator.clone()).or_default().insert(mint.clone());
            }
            LedgerFact::PoolLinked { mint, pool_id } => {
                self.mint_pool.insert(mint.clone(), pool_id.clone());
            }
            LedgerFact::Rugged { pool_id } => {
                self.rugged_pools.insert(pool_id.clone());
            }
            LedgerFact::HardBlocked { mint } => {
                self.flagged_mints.insert(mint.clone());
            }
        }
    }

    fn persist(&self, fact: &LedgerFact) {
        let Some(path) = &self.persist_path else { return };
        let line = serde_json::to_string(fact).expect("LedgerFact serialization cannot fail");
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(file, "{line}")
        })();
        if let Err(e) = result {
            eprintln!("creator ledger: failed to persist fact to {}: {e}", path.display());
        }
    }

    /// Records a new `TokenCreated` candidate from `creator`, and returns
    /// that creator's history score computed from their *prior* mints only
    /// (this new mint is appended after the score is read, so a creator's
    /// very first mint never counts itself). `None` means this process has
    /// never observed this creator launch anything before — the
    /// risk-engine's existing "unknown creator" handling
    /// (`probe_entry`, sized down, not blocked) is exactly the right
    /// response to that, not a defect of this ledger.
    pub fn observe_creation(&mut self, creator: &str, mint: &str) -> Option<f64> {
        let score = self.history_score(creator);
        let fact = LedgerFact::Creation { creator: creator.to_string(), mint: mint.to_string() };
        self.apply(&fact);
        self.persist(&fact);
        score
    }

    /// Links `mint` to the PumpSwap pool it graduated into, so a later
    /// `observe_liquidity_removed` on that pool can be attributed back to
    /// this mint (and from there, to its creator).
    pub fn observe_pool_created(&mut self, mint: &str, pool_id: &str) {
        let fact = LedgerFact::PoolLinked { mint: mint.to_string(), pool_id: pool_id.to_string() };
        self.apply(&fact);
        self.persist(&fact);
    }

    /// Records a real withdrawal's `all_liquidity_removed` outcome for
    /// `pool_id` — a no-op unless `all_liquidity_removed` is true (a
    /// partial withdrawal is normal LP activity, not rug evidence).
    pub fn observe_liquidity_removed(&mut self, pool_id: &str, all_liquidity_removed: bool) {
        if !all_liquidity_removed {
            return;
        }
        let fact = LedgerFact::Rugged { pool_id: pool_id.to_string() };
        self.apply(&fact);
        self.persist(&fact);
    }

    /// Records that `mint`'s own `TokenCreated` event was hard-blocked by
    /// `risk_engine` (a dangerous Token-2022 extension) — direct evidence
    /// against this mint's creator, independent of whether it ever reaches
    /// a PumpSwap pool at all.
    pub fn observe_hard_blocked(&mut self, mint: &str) {
        let fact = LedgerFact::HardBlocked { mint: mint.to_string() };
        self.apply(&fact);
        self.persist(&fact);
    }

    fn is_bad(&self, mint: &str) -> bool {
        self.flagged_mints.contains(mint) || self.mint_pool.get(mint).is_some_and(|pool| self.rugged_pools.contains(pool))
    }

    /// The fraction of `creator`'s known-outcome prior mints that were
    /// *not* flagged bad, in `[0.0, 1.0]`. `None` if this creator has no
    /// prior mints on record at all.
    fn history_score(&self, creator: &str) -> Option<f64> {
        let mints = self.creator_mints.get(creator)?;
        if mints.is_empty() {
            return None;
        }
        let bad = mints.iter().filter(|m| self.is_bad(m)).count();
        Some(1.0 - (bad as f64 / mints.len() as f64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_creators_first_ever_mint_has_no_history_score() {
        let mut ledger = CreatorLedger::in_memory();
        assert_eq!(ledger.observe_creation("creator-a", "mint-1"), None);
    }

    #[test]
    fn a_second_mint_from_a_clean_creator_scores_perfectly() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        // mint-1 never linked to a pool, never hard-blocked -> not bad.
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(1.0));
    }

    #[test]
    fn a_hard_blocked_prior_mint_lowers_the_next_score() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_hard_blocked("mint-1");
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(0.0));
    }

    #[test]
    fn a_graduated_and_fully_drained_prior_mint_lowers_the_next_score() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_pool_created("mint-1", "pool-1");
        ledger.observe_liquidity_removed("pool-1", true);
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(0.0));
    }

    #[test]
    fn a_partial_withdrawal_does_not_count_as_a_rug() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_pool_created("mint-1", "pool-1");
        ledger.observe_liquidity_removed("pool-1", false);
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(1.0));
    }

    #[test]
    fn score_is_the_fraction_of_prior_mints_that_were_not_bad() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_hard_blocked("mint-1");
        ledger.observe_creation("creator-a", "mint-2");
        ledger.observe_creation("creator-a", "mint-3");
        // Prior mints at this point: mint-1 (bad), mint-2 (clean), mint-3
        // (clean) -> 1/3 bad.
        let score = ledger.observe_creation("creator-a", "mint-4").unwrap();
        assert!((score - 2.0 / 3.0).abs() < 1e-9, "expected ~0.6667, got {score}");
    }

    /// Regression test for a real finding from independent review: a
    /// duplicate `Creation{creator, mint}` fact (a corrupted/duplicated
    /// persisted line, or any future caller invoking `observe_creation`
    /// twice for the same mint) must not double-count that mint in the
    /// denominator — `creator_mints` switched from `Vec` to `HashSet` to
    /// guarantee this.
    #[test]
    fn a_duplicate_creation_fact_does_not_inflate_the_mint_count() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_hard_blocked("mint-1");
        // Same (creator, mint) recorded again — must be a no-op on the count.
        ledger.observe_creation("creator-a", "mint-1");
        // If mint-1 were double-counted, the denominator here would be 2
        // (two "mint-1" entries) instead of 1, understating how bad this
        // creator's one real prior mint actually was.
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(0.0));
    }

    #[test]
    fn different_creators_have_independent_histories() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        ledger.observe_hard_blocked("mint-1");
        // creator-b has never been observed at all.
        assert_eq!(ledger.observe_creation("creator-b", "mint-2"), None);
    }

    #[test]
    fn a_rug_on_a_pool_not_linked_to_any_mint_affects_nothing() {
        let mut ledger = CreatorLedger::in_memory();
        ledger.observe_creation("creator-a", "mint-1");
        // No observe_pool_created call for mint-1: it never graduated.
        ledger.observe_liquidity_removed("some-unrelated-pool", true);
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(1.0));
    }

    #[test]
    fn persists_and_reloads_an_identical_history_score() {
        let dir = std::env::temp_dir().join(format!("creator_ledger_test_{}", std::process::id()));
        let path = dir.join("ledger.ndjson");
        let _ = fs::remove_dir_all(&dir);

        {
            let mut ledger = CreatorLedger::load(&path).unwrap();
            ledger.observe_creation("creator-a", "mint-1");
            ledger.observe_pool_created("mint-1", "pool-1");
            ledger.observe_liquidity_removed("pool-1", true);
        }

        let mut reloaded = CreatorLedger::load(&path).unwrap();
        assert_eq!(reloaded.observe_creation("creator-a", "mint-2"), Some(0.0));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loading_a_nonexistent_file_starts_a_fresh_empty_ledger() {
        let dir = std::env::temp_dir().join(format!("creator_ledger_test_missing_{}", std::process::id()));
        let path = dir.join("does_not_exist.ndjson");
        let _ = fs::remove_dir_all(&dir);

        let mut ledger = CreatorLedger::load(&path).unwrap();
        assert_eq!(ledger.observe_creation("creator-a", "mint-1"), None);

        let _ = fs::remove_dir_all(&dir);
    }

    /// Regression test for a real finding from independent review: a line
    /// that fails even to *read* as UTF-8 (not just one that reads fine
    /// but fails to parse as JSON) must not propagate an `Err` out of
    /// `load()` — that would hit the caller's `.expect()` in
    /// `bin/pipeline.rs`'s `main()` and panic the whole process on startup
    /// over a single torn write from a prior crash. Written with raw bytes
    /// directly, since the ledger's own writer never produces invalid
    /// UTF-8.
    #[test]
    fn an_unreadable_line_is_skipped_not_fatal() {
        let dir = std::env::temp_dir().join(format!("creator_ledger_test_badutf8_{}", std::process::id()));
        let path = dir.join("ledger.ndjson");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let good_line_before = serde_json::to_string(&LedgerFact::Creation { creator: "creator-a".to_string(), mint: "mint-1".to_string() }).unwrap();
        let good_line_after = serde_json::to_string(&LedgerFact::HardBlocked { mint: "mint-1".to_string() }).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(good_line_before.as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(&[0xFF, 0xFE, 0xFD]); // not valid UTF-8
        bytes.push(b'\n');
        bytes.extend_from_slice(good_line_after.as_bytes());
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();

        // Must not panic or return Err — the bad line is skipped, the two
        // good lines around it are still applied.
        let mut ledger = CreatorLedger::load(&path).expect("load must tolerate one unreadable line");
        assert_eq!(ledger.observe_creation("creator-a", "mint-2"), Some(0.0), "both surrounding good lines should have been applied");

        fs::remove_dir_all(&dir).unwrap();
    }
}
