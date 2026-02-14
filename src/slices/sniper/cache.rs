use std::{str::FromStr, sync::LazyLock};

use solana_sdk::pubkey::Pubkey;

use crate::adapters::raydium::{
    RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID, TOKEN_PROGRAM_ID, WSOL_ADDRESS,
};

const JITO_TIP_ACCOUNT_ADDRESS: &str = "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL";

static WSOL_PUBKEY: LazyLock<Option<Pubkey>> =
    LazyLock::new(|| Pubkey::from_str(WSOL_ADDRESS).ok());
static TOKEN_PROGRAM_PUBKEY: LazyLock<Option<Pubkey>> =
    LazyLock::new(|| Pubkey::from_str(TOKEN_PROGRAM_ID).ok());
static JITO_TIP_PUBKEY: LazyLock<Option<Pubkey>> =
    LazyLock::new(|| Pubkey::from_str(JITO_TIP_ACCOUNT_ADDRESS).ok());
static RAYDIUM_STANDARD_AMM_PROGRAM_PUBKEY: LazyLock<Option<Pubkey>> =
    LazyLock::new(|| Pubkey::from_str(RAYDIUM_STANDARD_AMM_PROGRAM_ID).ok());
static RAYDIUM_V4_PROGRAM_PUBKEY: LazyLock<Option<Pubkey>> =
    LazyLock::new(|| Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).ok());

#[inline(always)]
pub fn wsol_pubkey() -> Option<Pubkey> {
    WSOL_PUBKEY.as_ref().copied()
}

#[inline(always)]
pub fn token_program_pubkey() -> Option<Pubkey> {
    TOKEN_PROGRAM_PUBKEY.as_ref().copied()
}

#[inline(always)]
pub fn jito_tip_pubkey() -> Option<Pubkey> {
    JITO_TIP_PUBKEY.as_ref().copied()
}

#[inline(always)]
pub fn raydium_standard_amm_program_pubkey() -> Option<Pubkey> {
    RAYDIUM_STANDARD_AMM_PROGRAM_PUBKEY.as_ref().copied()
}

#[inline(always)]
pub fn raydium_v4_program_pubkey() -> Option<Pubkey> {
    RAYDIUM_V4_PROGRAM_PUBKEY.as_ref().copied()
}

#[cfg(test)]
mod tests {
    use super::{
        jito_tip_pubkey, raydium_standard_amm_program_pubkey, raydium_v4_program_pubkey,
        token_program_pubkey, wsol_pubkey,
    };

    #[test]
    fn parses_all_cached_pubkeys() {
        assert!(wsol_pubkey().is_some());
        assert!(token_program_pubkey().is_some());
        assert!(jito_tip_pubkey().is_some());
        assert!(raydium_standard_amm_program_pubkey().is_some());
        assert!(raydium_v4_program_pubkey().is_some());
    }
}
