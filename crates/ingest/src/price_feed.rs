//! Live SOL/USD price source: Pyth Network's on-chain "push" oracle
//! (`PriceUpdateV2`), read via `crates/live`'s `accountSubscribe` machinery.
//!
//! Chosen over the alternatives after actually testing them, not from
//! documentation alone:
//! - An external HTTP price API (CoinGecko/Jupiter) would add this
//!   project's first HTTPS dependency and a new polling pattern; Pyth's
//!   on-chain account reuses the `accountSubscribe`/`account_watcher`
//!   infrastructure `crates/live` already has for bonding curves and
//!   pools, with zero new networking code.
//! - Reading a single DEX pool's reserves directly (e.g. a SOL/USDC pool)
//!   was considered too, but ties the price to one specific pool's
//!   instantaneous, manipulable state; Pyth aggregates many venues off-chain
//!   before publishing.
//!
//! Pyth's oracle migrated to a "pull" model where most price feed accounts
//! are arbitrary, caller-derived PDAs that only update when *someone*
//! submits a fresh one — not a fixed, always-live address a passive
//! subscriber can just watch. The exception, confirmed against real
//! mainnet data below, is the small set of "sponsored" feeds the Pyth Data
//! Association keeps continuously updated at a fixed address — SOL/USD is
//! one of them.
//!
//! `PYTH_SOL_USD_PRICE_ACCOUNT` and the byte layout below were verified
//! against a real, live `getAccountInfo` snapshot of that account (not
//! just the SDK's Rust struct definitions, which don't publish the wire
//! discriminator or resolve the `VerificationLevel` enum's tag byte
//! explicitly): the account's own 8-byte discriminator matches
//! `sha256("account:PriceUpdateV2")[..8]` exactly, its embedded 32-byte
//! feed id matches the SOL/USD feed id from Pyth's own Hermes API
//! (`https://hermes.pyth.network/v2/price_feeds?query=SOL&asset_type=crypto`)
//! exactly, and the decoded price (~$94, checked twice a minute apart,
//! `publish_time` within seconds of wall-clock "now" both times) is a
//! plausible, fresh live SOL price — see `tests/price_feed_test.rs` for the
//! exact captured bytes this was verified against.

use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Fixed address of Pyth's continuously-updated ("sponsored push feed")
/// SOL/USD `PriceUpdateV2` account on Solana mainnet.
pub const PYTH_SOL_USD_PRICE_ACCOUNT: &str = "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE";

/// Pyth's Solana Receiver program — owns every `PriceUpdateV2` account,
/// including the one above. Checked defensively in `decode_sol_usd_update`
/// so a caller that accidentally points this at the wrong account (e.g. a
/// copy-paste of a different pubkey) gets a clear `None` instead of a
/// misparsed price.
pub const PYTH_RECEIVER_PROGRAM_ID: &str = "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ";

const PRICE_UPDATE_V2_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];

/// The SOL/USD feed id from Pyth's Hermes API, embedded inside the
/// account's `PriceFeedMessage` and checked on every decode — the account
/// address is hardcoded above, but this is a second, independent check
/// that the bytes really describe SOL/USD and not some other feed that
/// happened to reuse the address (defense in depth, not defense against a
/// specific known failure mode).
const SOL_USD_FEED_ID: [u8; 32] = [
    0xef, 0x0d, 0x8b, 0x6f, 0xda, 0x2c, 0xeb, 0xa4, 0x1d, 0xa1, 0x5d, 0x40, 0x95, 0xd1, 0xda, 0x39, 0x2a, 0x0d, 0x2f,
    0x8e, 0xd0, 0xc6, 0xc7, 0xbc, 0x0f, 0x4c, 0xfa, 0xc8, 0xc2, 0x80, 0xb5, 0x6d,
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PythPriceUpdate {
    /// SOL price in USD, already scaled by the feed's exponent.
    pub price: f64,
    /// Pyth's published confidence interval, in the same USD units as
    /// `price` — how far the true price could plausibly be from `price`,
    /// not a probability.
    pub conf: f64,
    /// Unix seconds this specific value was published, per the feed
    /// itself (not when this process observed it) — the basis for a
    /// caller's own staleness check.
    pub publish_time: i64,
}

/// Decodes a `PriceUpdateV2` account's raw bytes into a SOL/USD price,
/// given the account's owner (as reported by `accountSubscribe`/
/// `getAccountInfo` alongside the data). Returns `None` if the owner isn't
/// Pyth's Receiver program, the data doesn't parse as `PriceUpdateV2`, or
/// the embedded feed id isn't SOL/USD — any of those means this isn't the
/// account this module thinks it is, and a caller must not use whatever
/// numbers happened to decode from the wrong bytes.
pub fn decode_sol_usd_update(owner: &Pubkey, data: &[u8]) -> Option<PythPriceUpdate> {
    let receiver: Pubkey = PYTH_RECEIVER_PROGRAM_ID.parse().expect("PYTH_RECEIVER_PROGRAM_ID is a valid pubkey");
    if *owner != receiver {
        return None;
    }

    let mut r = Reader::new(data);
    let disc = r.bytes(8)?;
    if disc != PRICE_UPDATE_V2_DISCRIMINATOR {
        return None;
    }
    let _write_authority = r.pubkey()?;
    let verification_tag = r.u8()?;
    match verification_tag {
        0 => {
            // VerificationLevel::Partial { num_signatures: u8 } — one
            // extra byte this module doesn't need to read further.
            r.u8()?;
        }
        1 => {}
        // Neither of Anchor's two declared VerificationLevel variants —
        // something this module's byte-layout assumptions don't account
        // for, not safe to keep parsing past.
        _ => return None,
    }

    let feed_id = r.bytes(32)?;
    if feed_id != SOL_USD_FEED_ID {
        return None;
    }
    let price_raw = r.i64()?;
    let conf_raw = r.u64()?;
    let exponent = r.i32()?;
    let publish_time = r.i64()?;
    let _prev_publish_time = r.i64()?;
    let _ema_price_raw = r.i64()?;
    let _ema_conf_raw = r.u64()?;
    let _posted_slot = r.u64()?;

    let scale = 10f64.powi(exponent);
    let price = price_raw as f64 * scale;
    let conf = conf_raw as f64 * scale;
    if !price.is_finite() || price <= 0.0 || !conf.is_finite() {
        return None;
    }

    Some(PythPriceUpdate { price, conf, publish_time })
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

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.remaining() < len {
            return None;
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Some(slice)
    }

    fn u8(&mut self) -> Option<u8> {
        let b = self.bytes(1)?;
        Some(b[0])
    }

    fn u64(&mut self) -> Option<u64> {
        let b = self.bytes(8)?;
        Some(u64::from_le_bytes(b.try_into().unwrap()))
    }

    fn i64(&mut self) -> Option<i64> {
        let b = self.bytes(8)?;
        Some(i64::from_le_bytes(b.try_into().unwrap()))
    }

    fn i32(&mut self) -> Option<i32> {
        let b = self.bytes(4)?;
        Some(i32::from_le_bytes(b.try_into().unwrap()))
    }

    fn pubkey(&mut self) -> Option<Pubkey> {
        let b = self.bytes(32)?;
        let arr: [u8; 32] = b.try_into().unwrap();
        Some(Pubkey::new_from_array(arr))
    }
}

/// The account's own address, as a typed `Pubkey` — the value a live
/// pipeline `Watch`es on `account_watcher`.
pub fn sol_usd_price_account() -> Pubkey {
    Pubkey::from_str(PYTH_SOL_USD_PRICE_ACCOUNT).expect("PYTH_SOL_USD_PRICE_ACCOUNT is a valid pubkey")
}
