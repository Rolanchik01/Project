//! Fixtures captured live from mainnet during Stage 1 research (see the
//! module docs in src/lib.rs for why this wraps spl-token-2022 rather than
//! hand-parsing TLV data).

use base64::Engine;
use momentum_token2022::inspect_mint;

/// A real Pump-created mint (ExXQP6ZatSTMXpP7jaWcN76k8V9vwzFUD5rPNfWGpump —
/// the same mint used in the Pump bonding-curve fixtures), owned by the
/// Token-2022 program. It carries MetadataPointer/TokenMetadata (visible as
/// the readable "MarsCoin" / "https://m.rapidlaunch.io/..." strings in the
/// raw bytes below) but none of the dangerous extensions — this is what a
/// normal, tradeable Pump token's mint actually looks like today.
const REAL_PUMP_MINT_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAIDGpH6NAwAGAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAARIAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM9hQF+goN9boq87vpzHtXJMB6u9C5wSlLIpNzasFNPfEwCEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAz2FAX6Cg31uirzu+nMe1ckwHq70LnBKUsik3NqwU098IAAAATWFyc0NvaW4IAAAATWFyc2NvaW4kAAAAaHR0cHM6Ly9tLnJhcGlkbGF1bmNoLmlvL20vazVvU1V4YmJmAAAAAA==";

fn decode_fixture(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(b64).unwrap()
}

#[test]
fn a_real_pump_token_mint_has_no_dangerous_extensions() {
    let data = decode_fixture(REAL_PUMP_MINT_B64);
    assert_eq!(data.len(), 370, "must match the real mint account's on-chain space exactly");

    let flags = inspect_mint(&data).unwrap();
    assert!(!flags.transfer_hook);
    assert_eq!(flags.transfer_fee_bps, 0);
    assert!(!flags.permanent_delegate);
    assert!(!flags.non_transferable);
    assert!(!flags.default_frozen);
    assert!(!flags.has_restricted_transfer_mechanism());
    // The bonding-curve fixture for this same mint (crates/pump) has
    // mint/freeze authority both inactive; a graduated/live mint can have
    // its mint authority revoked at any point, so this only asserts the
    // fields are readable, not a specific value.
    let _ = (flags.mint_authority_active, flags.freeze_authority_active);
}

#[test]
fn detects_a_transfer_fee_and_transfer_hook_mint_built_with_the_official_packer() {
    use spl_token_2022::extension::transfer_fee::{TransferFee, TransferFeeConfig};
    use spl_token_2022::extension::transfer_hook::TransferHook;
    use spl_token_2022::extension::{BaseStateWithExtensionsMut, ExtensionType, PodStateWithExtensionsMut};
    use spl_pod::primitives::PodU16;
    use spl_token_2022::pod::{PodCOption, PodMint};

    let extensions = [ExtensionType::TransferFeeConfig, ExtensionType::TransferHook];
    let space = ExtensionType::try_calculate_account_len::<PodMint>(&extensions).unwrap();
    let mut buffer = vec![0u8; space];

    let mut state = PodStateWithExtensionsMut::<PodMint>::unpack_uninitialized(&mut buffer).unwrap();
    state.base.decimals = 6;
    state.base.is_initialized = true.into();
    state.base.mint_authority = PodCOption::none();
    state.base.freeze_authority = PodCOption::none();
    state.init_account_type().unwrap();

    let fee_config = state.init_extension::<TransferFeeConfig>(true).unwrap();
    let fee = TransferFee { epoch: 0.into(), maximum_fee: u64::MAX.into(), transfer_fee_basis_points: PodU16::from(500u16) };
    fee_config.older_transfer_fee = fee;
    fee_config.newer_transfer_fee = fee;

    state.init_extension::<TransferHook>(true).unwrap();

    let flags = momentum_token2022::inspect_mint(&buffer).unwrap();
    assert_eq!(flags.transfer_fee_bps, 500);
    assert!(flags.transfer_hook);
    assert!(flags.has_restricted_transfer_mechanism());
}
