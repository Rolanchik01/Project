//! Pump's Anchor self-CPI event log decoding: `CreateEvent`, `TradeEvent`,
//! `CompleteEvent`. `CreateEvent` and `TradeEvent` (both `is_buy` values)
//! are verified byte-for-byte against real mainnet transactions — see
//! `tests/events_test.rs`. `CompleteEvent` (graduation) uses the exact same
//! decode methodology, applied consistently, but no real graduation was
//! captured during Stage 1 research (mainnet migrations are rare enough
//! that a wide signature search came up empty); it should be re-verified
//! against a real one the first time the recorder observes it live.

use solana_pubkey::Pubkey;

const CREATE_EVENT_DISCRIMINATOR: [u8; 8] = [27, 114, 169, 77, 222, 235, 99, 118];
const TRADE_EVENT_DISCRIMINATOR: [u8; 8] = [189, 219, 127, 211, 78, 230, 97, 238];
const COMPLETE_EVENT_DISCRIMINATOR: [u8; 8] = [95, 114, 97, 156, 212, 46, 152, 8];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateEvent {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub creator: Pubkey,
    pub timestamp: i64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub token_total_supply: u64,
    pub token_program: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_enabled: bool,
    pub quote_mint: Pubkey,
    pub virtual_quote_reserves: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeEvent {
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub is_buy: bool,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub fee_recipient: Pubkey,
    pub fee_basis_points: u64,
    pub fee: u64,
    pub creator: Pubkey,
    pub creator_fee_basis_points: u64,
    pub creator_fee: u64,
    pub ix_name: String,
    pub mayhem_mode: bool,
    pub quote_mint: Pubkey,
    pub quote_amount: u64,
    pub virtual_quote_reserves: u64,
    pub real_quote_reserves: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompleteEvent {
    pub user: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub timestamp: i64,
    pub quote_mint: Pubkey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PumpEvent {
    Create(CreateEvent),
    Trade(TradeEvent),
    Complete(CompleteEvent),
}

/// Decodes one Anchor self-CPI event log (8-byte discriminator + payload).
/// Returns `None` for anything that isn't one of the three event kinds
/// this bot needs (there are ~30 other event/instruction types this
/// program emits — cashback claims, admin config changes, and so on —
/// that carry no trading signal and are deliberately not decoded).
pub fn decode_event(data: &[u8]) -> Option<PumpEvent> {
    if data.len() < 8 {
        return None;
    }
    let (disc, body) = data.split_at(8);
    match disc {
        d if d == CREATE_EVENT_DISCRIMINATOR => decode_create(body).map(PumpEvent::Create),
        d if d == TRADE_EVENT_DISCRIMINATOR => decode_trade(body).map(PumpEvent::Trade),
        d if d == COMPLETE_EVENT_DISCRIMINATOR => decode_complete(body).map(PumpEvent::Complete),
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
        let len = self.u64_as_u32()?;
        if self.remaining() < len as usize {
            return None;
        }
        let s = String::from_utf8(self.data[self.pos..self.pos + len as usize].to_vec()).ok()?;
        self.pos += len as usize;
        Some(s)
    }

    fn u64_as_u32(&mut self) -> Option<u32> {
        if self.remaining() < 4 {
            return None;
        }
        let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Some(v)
    }
}

fn decode_create(body: &[u8]) -> Option<CreateEvent> {
    let mut r = Reader::new(body);
    Some(CreateEvent {
        name: r.string()?,
        symbol: r.string()?,
        uri: r.string()?,
        mint: r.pubkey()?,
        bonding_curve: r.pubkey()?,
        user: r.pubkey()?,
        creator: r.pubkey()?,
        timestamp: r.i64()?,
        virtual_token_reserves: r.u64()?,
        virtual_sol_reserves: r.u64()?,
        real_token_reserves: r.u64()?,
        token_total_supply: r.u64()?,
        token_program: r.pubkey()?,
        is_mayhem_mode: r.bool()?,
        is_cashback_enabled: r.bool()?,
        quote_mint: r.pubkey()?,
        virtual_quote_reserves: r.u64()?,
    })
}

fn decode_trade(body: &[u8]) -> Option<TradeEvent> {
    let mut r = Reader::new(body);
    let mint = r.pubkey()?;
    let sol_amount = r.u64()?;
    let token_amount = r.u64()?;
    let is_buy = r.bool()?;
    let user = r.pubkey()?;
    let timestamp = r.i64()?;
    let virtual_sol_reserves = r.u64()?;
    let virtual_token_reserves = r.u64()?;
    let real_sol_reserves = r.u64()?;
    let real_token_reserves = r.u64()?;
    let fee_recipient = r.pubkey()?;
    let fee_basis_points = r.u64()?;
    let fee = r.u64()?;
    let creator = r.pubkey()?;
    let creator_fee_basis_points = r.u64()?;
    let creator_fee = r.u64()?;
    let _track_volume = r.bool()?;
    let _total_unclaimed_tokens = r.u64()?;
    let _total_claimed_tokens = r.u64()?;
    let _current_sol_volume = r.u64()?;
    let _last_update_timestamp = r.i64()?;
    let ix_name = r.string()?;
    let mayhem_mode = r.bool()?;
    let _cashback_fee_basis_points = r.u64()?;
    let _cashback = r.u64()?;
    let _buyback_fee_basis_points = r.u64()?;
    let _buyback_fee = r.u64()?;
    let shareholders_len = r.u64_as_u32()?;
    // Shareholder payout splits: not used for pricing/scoring, skip over.
    for _ in 0..shareholders_len {
        r.pubkey()?;
        r.u64()?;
    }
    let quote_mint = r.pubkey()?;
    let quote_amount = r.u64()?;
    let virtual_quote_reserves = r.u64()?;
    let real_quote_reserves = r.u64()?;

    Some(TradeEvent {
        mint,
        sol_amount,
        token_amount,
        is_buy,
        user,
        timestamp,
        virtual_sol_reserves,
        virtual_token_reserves,
        real_sol_reserves,
        real_token_reserves,
        fee_recipient,
        fee_basis_points,
        fee,
        creator,
        creator_fee_basis_points,
        creator_fee,
        ix_name,
        mayhem_mode,
        quote_mint,
        quote_amount,
        virtual_quote_reserves,
        real_quote_reserves,
    })
}

fn decode_complete(body: &[u8]) -> Option<CompleteEvent> {
    let mut r = Reader::new(body);
    Some(CompleteEvent {
        user: r.pubkey()?,
        mint: r.pubkey()?,
        bonding_curve: r.pubkey()?,
        timestamp: r.i64()?,
        quote_mint: r.pubkey()?,
    })
}
