//! Wallet/creator intelligence (Group E): reputation this process has
//! itself observed on-chain, not fetched from a third party or crawled
//! from full historical archaeology — see [`creator`]'s and [`wallet`]'s
//! module doc comments for why, in each case. Both feed the previously
//! always-`None` `creator_history_score`/`buyer_cluster_id`/
//! `buyer_quality`/`seller_cluster_id` fields `core::domain::Event` has
//! carried since Stage 0.

pub mod creator;
pub mod wallet;

pub use creator::CreatorLedger;
pub use wallet::TraderLedger;
