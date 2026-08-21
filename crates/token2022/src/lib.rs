//! Token-2022 mint extension inspection for the hard-veto gate.
//!
//! Deliberately does not hand-roll TLV parsing: extension layout has real
//! footguns (e.g. Token-2022 inserts a padding byte when an extended mint
//! would otherwise collide in length with a legacy 165-byte token account)
//! that are easy to get subtly wrong. This wraps the official
//! `spl-token-2022` crate's `StateWithExtensions` decoder instead, so this
//! module's job is just mapping its typed extensions onto the fields
//! `risk-engine`'s hard-veto list already checks.

use spl_token_2022::extension::default_account_state::DefaultAccountState;
use spl_token_2022::extension::permanent_delegate::PermanentDelegate;
use spl_token_2022::extension::transfer_fee::TransferFeeConfig;
use spl_token_2022::extension::transfer_hook::TransferHook;
use spl_token_2022::extension::{BaseStateWithExtensions, StateWithExtensions};
use spl_token_2022::state::{AccountState, Mint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectError {
    Unpack,
}

/// Maps directly onto the fields `risk-engine`'s `hardBlocks`/`TechnicalFlags`
/// already understand (see crates/core/src/risk_engine.rs and domain.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MintExtensionFlags {
    pub mint_authority_active: bool,
    pub freeze_authority_active: bool,
    pub transfer_hook: bool,
    pub transfer_fee_bps: u32,
    pub permanent_delegate: bool,
    pub non_transferable: bool,
    /// True if new token accounts for this mint are frozen by default —
    /// functionally equivalent to an active freeze authority for veto
    /// purposes (holders cannot move the token without an unfreeze first).
    pub default_frozen: bool,
}

impl MintExtensionFlags {
    /// True if any flag here is itself sufficient reason for a hard veto
    /// (transfer hook, nonzero transfer fee, permanent delegate,
    /// non-transferable, or default-frozen accounts) — mint/freeze
    /// authority are reported as plain fields since Stage 0's hardBlocks
    /// already has its own veto for those, shared with legacy SPL Token
    /// mints.
    pub fn has_restricted_transfer_mechanism(&self) -> bool {
        self.transfer_hook
            || self.transfer_fee_bps > 0
            || self.permanent_delegate
            || self.non_transferable
            || self.default_frozen
    }
}

pub fn inspect_mint(data: &[u8]) -> Result<MintExtensionFlags, InspectError> {
    let state = StateWithExtensions::<Mint>::unpack(data).map_err(|_| InspectError::Unpack)?;

    let mut flags = MintExtensionFlags {
        mint_authority_active: state.base.mint_authority.is_some(),
        freeze_authority_active: state.base.freeze_authority.is_some(),
        ..MintExtensionFlags::default()
    };

    if let Ok(transfer_fee) = state.get_extension::<TransferFeeConfig>() {
        let newer_bps = u16::from(transfer_fee.newer_transfer_fee.transfer_fee_basis_points);
        let older_bps = u16::from(transfer_fee.older_transfer_fee.transfer_fee_basis_points);
        flags.transfer_fee_bps = newer_bps.max(older_bps) as u32;
    }

    if state.get_extension::<TransferHook>().is_ok() {
        flags.transfer_hook = true;
    }

    if state.get_extension::<PermanentDelegate>().is_ok() {
        flags.permanent_delegate = true;
    }

    if state.get_extension::<spl_token_2022::extension::non_transferable::NonTransferable>().is_ok() {
        flags.non_transferable = true;
    }

    if let Ok(default_state) = state.get_extension::<DefaultAccountState>() {
        flags.default_frozen = default_state.state == u8::from(AccountState::Frozen);
    }

    Ok(flags)
}
