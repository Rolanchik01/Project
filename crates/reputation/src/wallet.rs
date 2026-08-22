//! Buyer/seller clustering and quality — the other half of "unknown
//! wallets get scrutiny, not bans", alongside [`crate::creator`].
//!
//! `core::domain::EventPayload::Buy`'s `buyer_cluster_id`/`buyer_quality`
//! feed `risk_engine`'s `strong_clusters` count directly (see
//! `risk_engine.rs`: a cluster only counts as "strong" once
//! `quality >= strong_cluster_quality_threshold && net_buy_usd > 0.0`) —
//! the signal `ConfirmedEntry` partly gates on is "how many *independent*
//! high-quality buyers does this token have", which is exactly the number
//! a Sybil ring (one actor funding N fresh wallets to fake organic demand)
//! is designed to inflate. Two real, zero-extra-RPC-call signals are used
//! against that, both intentionally conservative:
//!
//! - **Clustering**: wallets that buy the *same mint* in the *same slot*
//!   share a cluster id. A single actor's script firing N buys from N
//!   wallets in one transaction batch lands in one slot; genuinely
//!   independent buyers arriving over time do not. This can only ever
//!   *reduce* the apparent cluster count relative to "every wallet is its
//!   own cluster" — it cannot manufacture false diversity, only collapse
//!   real coordination that would otherwise inflate it.
//! - **Quality**: a wallet earns quality by trading across multiple
//!   distinct *instrument keys* this process has observed, with a bonus
//!   for having sold before (a real position round-trip, not just a
//!   one-way pump). A wallet's first-ever observed trade always has
//!   quality `0.0` — deliberately below any plausible threshold, the same
//!   "no evidence yet is not evidence of trustworthiness" posture
//!   `creator`'s ledger takes.
//!
//! "Instrument key" is deliberately not always a mint: `bin/pipeline.rs`
//! calls `observe_trade` keyed by mint for Pump trades but by *pool
//! address* for PumpSwap trades (which side of a pool is SOL isn't
//! resolved until the pool's cached state is looked up, one call deeper
//! than where the ledger key is needed — see that call site's comment).
//! Consequence, not just a hypothetical: a wallet that trades one token
//! both pre-graduation (Pump, keyed by mint) and post-graduation
//! (PumpSwap, keyed by the new pool) is counted as having traded *two*
//! distinct instruments, not one, inflating its diversity term by up to
//! `1 / QUALITY_MINT_DIVERSITY_CAP` per such token. Harmless for
//! same-slot *clustering* (a pool address is just as valid a coordination
//! key there), a real if modest impurity for *quality*'s "how many
//! genuinely distinct assets has this wallet traded" — accepted rather
//! than threading the resolved mint back out of `ingest_pumpswap_trade`
//! just to fix a few hundredths of a point.
//!
//! # What this deliberately does not attempt
//!
//! A funding-source graph check (do two "independent" wallets trace back
//! to the same first SOL deposit?) is a stronger clustering signal than
//! same-slot co-buying, but it requires a `getSignaturesForAddress` +
//! `getTransaction` round-trip per *new* wallet — tractable only for the
//! handful of wallets buying a token in its first seconds, not for every
//! trade this process sees (hundreds per minute in live testing). Left
//! for a later pass; not attempted here rather than attempted and left
//! unverified.
//!
//! # Verification note
//!
//! As with `creator`, there is no external ground truth to check this
//! module's formula against — it is this project's own policy. Tests
//! verify the policy behaves as designed, not that it matches a reference.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A trader's activity worth remembering across a restart. Same rationale
/// as `creator::LedgerFact`: `core::domain::Event` never carried the raw
/// wallet pubkey (only the derived `buyer_cluster_id`/`buyer_quality`), so
/// this is this module's own persistence, replayed independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
enum WalletFact {
    Trade { wallet: String, mint: String, is_buy: bool },
}

/// Distinct mints a wallet needs to have traded, on top of having sold at
/// least once, to reach `1.0` quality — see `quality`'s doc comment.
const QUALITY_MINT_DIVERSITY_CAP: usize = 5;
/// Quality contributed once a wallet has ever sold (a real round-trip),
/// independent of how many mints it's traded.
const QUALITY_SOLD_BONUS: f64 = 0.3;
/// Remaining quality budget split across mint diversity, capped at
/// `QUALITY_MINT_DIVERSITY_CAP` distinct mints.
const QUALITY_DIVERSITY_BUDGET: f64 = 1.0 - QUALITY_SOLD_BONUS;

#[derive(Debug, Default)]
pub struct TraderLedger {
    wallet_mints: HashMap<String, HashSet<String>>,
    wallet_has_sold: HashSet<String>,
    /// `(mint, slot) -> cluster_id`, assigned to the first wallet observed
    /// buying that mint in that slot. Deliberately **not** persisted:
    /// unlike trading history, "which wallets co-bought in one slot" isn't
    /// a fact worth carrying across a restart — see module doc comment.
    slot_clusters: HashMap<(String, u64), String>,
    persist_path: Option<PathBuf>,
}

impl TraderLedger {
    /// A ledger that persists nothing — every fact lives only for this
    /// process's lifetime. Useful for tests and for callers that manage
    /// their own persistence.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Loads previously-persisted wallet trading history from `path` (a
    /// fresh deployment with no history yet is not an error), then keeps
    /// appending new facts to the same file as they're observed. A line
    /// that fails to parse is skipped with a warning rather than aborting
    /// startup, same reasoning as `CreatorLedger::load`.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut ledger = Self { persist_path: Some(path.clone()), ..Self::default() };

        match fs::File::open(&path) {
            Ok(file) => {
                for (line_no, line) in io::BufReader::new(file).lines().enumerate() {
                    // See CreatorLedger::load's matching comment: a line
                    // that fails even to read (not just to parse) must not
                    // abort the whole load via `?` — that's exactly the
                    // "one bad line, not the whole history" failure this
                    // loop exists to prevent.
                    let line = match line {
                        Ok(line) => line,
                        Err(e) => {
                            eprintln!("trader ledger: skipping unreadable line at {}:{}: {e}", path.display(), line_no + 1);
                            continue;
                        }
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<WalletFact>(&line) {
                        Ok(fact) => ledger.apply(&fact),
                        Err(e) => eprintln!("trader ledger: skipping unparseable fact at {}:{}: {e}", path.display(), line_no + 1),
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }

        Ok(ledger)
    }

    fn apply(&mut self, fact: &WalletFact) {
        let WalletFact::Trade { wallet, mint, is_buy } = fact;
        self.wallet_mints.entry(wallet.clone()).or_default().insert(mint.clone());
        if !is_buy {
            self.wallet_has_sold.insert(wallet.clone());
        }
    }

    fn persist(&self, fact: &WalletFact) {
        let Some(path) = &self.persist_path else { return };
        let line = serde_json::to_string(fact).expect("WalletFact serialization cannot fail");
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(file, "{line}")
        })();
        if let Err(e) = result {
            eprintln!("trader ledger: failed to persist fact to {}: {e}", path.display());
        }
    }

    /// Records one real trade and returns the `(cluster_id, buyer_quality)`
    /// this trade should carry into its `Buy`/`Sell` event — both computed
    /// from state *before* this trade is recorded (so a wallet's very
    /// first trade can't inflate its own quality or invent a cluster
    /// partner out of itself).
    pub fn observe_trade(&mut self, wallet: &str, mint: &str, slot: u64, is_buy: bool) -> (String, f64) {
        let quality = self.quality(wallet);
        let cluster_id = self.slot_clusters.entry((mint.to_string(), slot)).or_insert_with(|| wallet.to_string()).clone();

        let fact = WalletFact::Trade { wallet: wallet.to_string(), mint: mint.to_string(), is_buy };
        self.apply(&fact);
        self.persist(&fact);

        (cluster_id, quality)
    }

    /// `0.0` for a wallet this process has never seen trade before (its
    /// current trade doesn't count — this is queried *before* recording
    /// it). Otherwise, up to `QUALITY_DIVERSITY_BUDGET` scaled by distinct
    /// prior mints traded (capped at `QUALITY_MINT_DIVERSITY_CAP`), plus
    /// `QUALITY_SOLD_BONUS` if this wallet has ever sold. A wallet needs
    /// both meaningful diversity *and* a real round-trip to clear
    /// `strong_cluster_quality_threshold` (`0.75` in
    /// `DEFAULT_SCORING_CONFIG`) — trading many mints without ever selling,
    /// or selling once with no other history, isn't enough on its own.
    fn quality(&self, wallet: &str) -> f64 {
        let Some(mints) = self.wallet_mints.get(wallet) else { return 0.0 };
        let diversity = (mints.len().min(QUALITY_MINT_DIVERSITY_CAP) as f64 / QUALITY_MINT_DIVERSITY_CAP as f64) * QUALITY_DIVERSITY_BUDGET;
        let sold_bonus = if self.wallet_has_sold.contains(wallet) { QUALITY_SOLD_BONUS } else { 0.0 };
        (diversity + sold_bonus).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wallets_first_ever_trade_has_zero_quality() {
        let mut ledger = TraderLedger::in_memory();
        let (_, quality) = ledger.observe_trade("wallet-a", "mint-1", 100, true);
        assert_eq!(quality, 0.0);
    }

    #[test]
    fn quality_grows_with_distinct_mints_traded() {
        let mut ledger = TraderLedger::in_memory();
        ledger.observe_trade("wallet-a", "mint-1", 100, true);
        let (_, q2) = ledger.observe_trade("wallet-a", "mint-2", 200, true);
        ledger.observe_trade("wallet-a", "mint-2", 200, true);
        let (_, q3) = ledger.observe_trade("wallet-a", "mint-3", 300, true);
        assert!(q3 > q2, "quality should strictly increase with more distinct mints: q2={q2} q3={q3}");
        assert!(q2 > 0.0);
    }

    #[test]
    fn selling_grants_a_quality_bonus_independent_of_mint_diversity() {
        let mut ledger = TraderLedger::in_memory();
        ledger.observe_trade("wallet-a", "mint-1", 100, true);
        let (_, before_sell) = ledger.observe_trade("wallet-a", "mint-1", 150, false);
        ledger.observe_trade("wallet-a", "mint-1", 150, false);
        let (_, after_sell) = ledger.observe_trade("wallet-a", "mint-1", 200, true);
        assert!(after_sell > before_sell, "quality should rise once this wallet has sold: before={before_sell} after={after_sell}");
    }

    #[test]
    fn enough_diversity_and_a_sale_clears_the_strong_cluster_threshold() {
        let mut ledger = TraderLedger::in_memory();
        for i in 0..4 {
            ledger.observe_trade("wallet-a", &format!("mint-{i}"), i as u64, true);
        }
        ledger.observe_trade("wallet-a", "mint-0", 100, false);
        let (_, quality) = ledger.observe_trade("wallet-a", "mint-99", 999, true);
        assert!(quality >= 0.75, "expected quality to clear the real strong_cluster_quality_threshold (0.75), got {quality}");
    }

    #[test]
    fn quality_never_exceeds_one() {
        let mut ledger = TraderLedger::in_memory();
        for i in 0..20 {
            ledger.observe_trade("wallet-a", &format!("mint-{i}"), i as u64, true);
            ledger.observe_trade("wallet-a", &format!("mint-{i}"), i as u64 + 1000, false);
        }
        let (_, quality) = ledger.observe_trade("wallet-a", "mint-final", 9999, true);
        assert!(quality <= 1.0);
    }

    #[test]
    fn wallets_buying_the_same_mint_in_the_same_slot_share_a_cluster() {
        let mut ledger = TraderLedger::in_memory();
        let (cluster_a, _) = ledger.observe_trade("wallet-a", "mint-1", 500, true);
        let (cluster_b, _) = ledger.observe_trade("wallet-b", "mint-1", 500, true);
        assert_eq!(cluster_a, cluster_b, "same mint, same slot -> same cluster");
    }

    #[test]
    fn wallets_buying_the_same_mint_in_different_slots_get_different_clusters() {
        let mut ledger = TraderLedger::in_memory();
        let (cluster_a, _) = ledger.observe_trade("wallet-a", "mint-1", 500, true);
        let (cluster_b, _) = ledger.observe_trade("wallet-b", "mint-1", 501, true);
        assert_ne!(cluster_a, cluster_b, "different slots should not be merged into one cluster");
    }

    #[test]
    fn the_same_slot_on_different_mints_does_not_merge_clusters() {
        let mut ledger = TraderLedger::in_memory();
        let (cluster_a, _) = ledger.observe_trade("wallet-a", "mint-1", 500, true);
        let (cluster_b, _) = ledger.observe_trade("wallet-b", "mint-2", 500, true);
        assert_ne!(cluster_a, cluster_b, "same slot but different mints must not be treated as coordinated");
    }

    #[test]
    fn persists_and_reloads_identical_quality() {
        let dir = std::env::temp_dir().join(format!("trader_ledger_test_{}", std::process::id()));
        let path = dir.join("ledger.ndjson");
        let _ = fs::remove_dir_all(&dir);

        {
            let mut ledger = TraderLedger::load(&path).unwrap();
            ledger.observe_trade("wallet-a", "mint-1", 100, true);
            ledger.observe_trade("wallet-a", "mint-1", 150, false);
        }

        let mut reloaded = TraderLedger::load(&path).unwrap();
        let (_, quality) = reloaded.observe_trade("wallet-a", "mint-2", 200, true);
        assert!(quality > 0.0, "reloaded ledger should remember wallet-a's prior mint-1 activity");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn slot_clustering_does_not_survive_a_reload() {
        let dir = std::env::temp_dir().join(format!("trader_ledger_test_noclust_{}", std::process::id()));
        let path = dir.join("ledger.ndjson");
        let _ = fs::remove_dir_all(&dir);

        {
            let mut ledger = TraderLedger::load(&path).unwrap();
            ledger.observe_trade("wallet-a", "mint-1", 500, true);
        }

        // A fresh process reloading the same file, seeing wallet-b buy the
        // exact same (mint, slot) wallet-a used before restart, must not
        // retroactively cluster them — that in-memory assignment is gone.
        let mut reloaded = TraderLedger::load(&path).unwrap();
        let (cluster_b, _) = reloaded.observe_trade("wallet-b", "mint-1", 500, true);
        assert_eq!(cluster_b, "wallet-b", "slot clustering must start fresh after a reload");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Regression test for a real finding from independent review — see
    /// `creator::tests::an_unreadable_line_is_skipped_not_fatal`'s doc
    /// comment for the full reasoning; same fix, same risk, this module's
    /// own `load()`.
    #[test]
    fn an_unreadable_line_is_skipped_not_fatal() {
        let dir = std::env::temp_dir().join(format!("trader_ledger_test_badutf8_{}", std::process::id()));
        let path = dir.join("ledger.ndjson");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let good_line = serde_json::to_string(&WalletFact::Trade { wallet: "wallet-a".to_string(), mint: "mint-1".to_string(), is_buy: true }).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0xFF, 0xFE, 0xFD]); // not valid UTF-8
        bytes.push(b'\n');
        bytes.extend_from_slice(good_line.as_bytes());
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();

        let mut ledger = TraderLedger::load(&path).expect("load must tolerate one unreadable line");
        let (_, quality) = ledger.observe_trade("wallet-a", "mint-2", 100, true);
        assert!(quality > 0.0, "the good line after the bad one should still have been applied");

        fs::remove_dir_all(&dir).unwrap();
    }
}
