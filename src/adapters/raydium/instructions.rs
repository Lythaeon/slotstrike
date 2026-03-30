use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use solana_address_lookup_table_interface::state::AddressLookupTable;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_message::{
    AccountKeys, VersionedMessage, v0::LoadedAddresses, v0::MessageAddressTableLookup,
};
use solana_sdk::{
    message::compiled_instruction::CompiledInstruction, pubkey::Pubkey,
    transaction::VersionedTransaction,
};
use tokio::sync::RwLock;

use super::constants::{
    RAYDIUM_V4_PROGRAM_ID, STANDARD_AMM_INITIALIZE, STANDARD_AMM_INITIALIZE_WITH_PERMISSION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RaydiumStructuredCandidateKind {
    Cpmm,
    OpenBook,
}

pub const RAYDIUM_V4_INITIALIZE_TAG: u8 = 0;
pub const RAYDIUM_V4_INITIALIZE2_TAG: u8 = 1;
pub const RAYDIUM_V4_SWAP_BASE_IN_TAG: u8 = 9;
pub const RAYDIUM_V4_SWAP_BASE_OUT_TAG: u8 = 11;

type LookupTableCache = RwLock<HashMap<Pubkey, Arc<[Pubkey]>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedCpmmCreation {
    pub deployer_address: Pubkey,
    pub amm_config: Pubkey,
    pub authority: Pubkey,
    pub pool_state: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub observation_state: Pubkey,
    pub token_program_a: Pubkey,
    pub token_program_b: Pubkey,
    pub init_amount_0: u64,
    pub init_amount_1: u64,
    pub open_time: u64,
}

impl ParsedCpmmCreation {
    #[inline(always)]
    pub fn token_mint(self) -> Option<Pubkey> {
        match (self.mint_a == wsol_pubkey(), self.mint_b == wsol_pubkey()) {
            (true, false) => Some(self.mint_b),
            (false, true) => Some(self.mint_a),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn token_program(self) -> Option<Pubkey> {
        match (self.mint_a == wsol_pubkey(), self.mint_b == wsol_pubkey()) {
            (true, false) => Some(self.token_program_b),
            (false, true) => Some(self.token_program_a),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn token_is_vault_zero(self) -> bool {
        self.mint_a != wsol_pubkey()
    }

    #[inline(always)]
    pub fn input_vault(self) -> Pubkey {
        if self.token_is_vault_zero() {
            self.vault_b
        } else {
            self.vault_a
        }
    }

    #[inline(always)]
    pub fn output_vault(self) -> Pubkey {
        if self.token_is_vault_zero() {
            self.vault_a
        } else {
            self.vault_b
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedOpenbookCreation {
    pub id: Pubkey,
    pub authority: Pubkey,
    pub open_orders: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub target_orders: Pubkey,
    pub market_program_id: Pubkey,
    pub market_id: Pubkey,
    pub deployer_address: Pubkey,
    pub init_pc_amount: u64,
    pub init_coin_amount: u64,
    pub open_time: i64,
}

impl ParsedOpenbookCreation {
    #[inline(always)]
    pub fn token_mint(self) -> Option<Pubkey> {
        match (self.mint_a == wsol_pubkey(), self.mint_b == wsol_pubkey()) {
            (true, false) => Some(self.mint_b),
            (false, true) => Some(self.mint_a),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn token_is_coin_mint(self) -> bool {
        self.mint_a != wsol_pubkey()
    }
}

#[inline(always)]
pub fn classify_raydium_creation_instructions(
    static_account_keys: &[Pubkey],
    instructions: &[CompiledInstruction],
    cpmm_program: Pubkey,
    openbook_program: Pubkey,
) -> Option<RaydiumStructuredCandidateKind> {
    for instruction in instructions {
        let Some(program_id) = static_account_keys.get(usize::from(instruction.program_id_index))
        else {
            continue;
        };

        if *program_id == cpmm_program && is_cpmm_creation_instruction(&instruction.data) {
            return Some(RaydiumStructuredCandidateKind::Cpmm);
        }

        if *program_id == openbook_program && is_openbook_creation_instruction(&instruction.data) {
            return Some(RaydiumStructuredCandidateKind::OpenBook);
        }
    }

    None
}

#[inline(always)]
pub fn is_cpmm_creation_instruction(data: &[u8]) -> bool {
    let Some(discriminator) = data.get(..STANDARD_AMM_INITIALIZE.len()) else {
        return false;
    };

    discriminator == STANDARD_AMM_INITIALIZE
        || discriminator == STANDARD_AMM_INITIALIZE_WITH_PERMISSION
}

#[inline(always)]
pub const fn is_openbook_creation_instruction(data: &[u8]) -> bool {
    matches!(data.first().copied(), Some(RAYDIUM_V4_INITIALIZE2_TAG))
}

pub const fn raydium_v4_program_pubkey() -> Pubkey {
    Pubkey::from_str_const(RAYDIUM_V4_PROGRAM_ID)
}

pub async fn parse_cpmm_creation_transaction(
    rpc: &RpcClient,
    tx: &VersionedTransaction,
    cpmm_program: Pubkey,
) -> Option<ParsedCpmmCreation> {
    let resolved_keys = resolve_account_keys(rpc, tx).await?;

    for instruction in tx.message.instructions() {
        let program_id = resolved_keys.get(usize::from(instruction.program_id_index))?;
        if *program_id != cpmm_program || !is_cpmm_creation_instruction(&instruction.data) {
            continue;
        }

        let accounts = resolve_instruction_accounts(&resolved_keys, instruction)?;
        let (init_amount_0, init_amount_1, open_time) =
            parse_cpmm_creation_data(&instruction.data)?;

        return Some(ParsedCpmmCreation {
            deployer_address: *accounts.first()?,
            amm_config: *accounts.get(1)?,
            authority: *accounts.get(2)?,
            pool_state: *accounts.get(3)?,
            mint_a: *accounts.get(4)?,
            mint_b: *accounts.get(5)?,
            vault_a: *accounts.get(10)?,
            vault_b: *accounts.get(11)?,
            observation_state: *accounts.get(13)?,
            token_program_a: *accounts.get(15)?,
            token_program_b: *accounts.get(16)?,
            init_amount_0,
            init_amount_1,
            open_time,
        });
    }

    None
}

pub async fn parse_openbook_creation_transaction(
    rpc: &RpcClient,
    tx: &VersionedTransaction,
    openbook_program: Pubkey,
) -> Option<ParsedOpenbookCreation> {
    let resolved_keys = resolve_account_keys(rpc, tx).await?;

    for instruction in tx.message.instructions() {
        let program_id = resolved_keys.get(usize::from(instruction.program_id_index))?;
        if *program_id != openbook_program || !is_openbook_creation_instruction(&instruction.data) {
            continue;
        }

        let accounts = resolve_instruction_accounts(&resolved_keys, instruction)?;
        let (init_pc_amount, init_coin_amount, open_time) =
            parse_openbook_creation_data(&instruction.data)?;

        return Some(ParsedOpenbookCreation {
            id: *accounts.get(4)?,
            authority: *accounts.get(5)?,
            open_orders: *accounts.get(6)?,
            mint_a: *accounts.get(8)?,
            mint_b: *accounts.get(9)?,
            base_vault: *accounts.get(10)?,
            quote_vault: *accounts.get(11)?,
            target_orders: *accounts.get(12)?,
            market_program_id: *accounts.get(15)?,
            market_id: *accounts.get(16)?,
            deployer_address: *accounts.get(17)?,
            init_pc_amount,
            init_coin_amount,
            open_time,
        });
    }

    None
}

async fn resolve_account_keys(rpc: &RpcClient, tx: &VersionedTransaction) -> Option<Vec<Pubkey>> {
    match &tx.message {
        VersionedMessage::Legacy(message) => Some(message.account_keys.clone()),
        VersionedMessage::V0(message) => {
            let loaded_addresses =
                load_lookup_table_addresses(rpc, &message.address_table_lookups).await?;
            let account_keys = AccountKeys::new(&message.account_keys, Some(&loaded_addresses));
            Some(account_keys.iter().copied().collect())
        }
    }
}

async fn load_lookup_table_addresses(
    rpc: &RpcClient,
    lookups: &[MessageAddressTableLookup],
) -> Option<LoadedAddresses> {
    if lookups.is_empty() {
        return Some(LoadedAddresses::default());
    }

    let mut resolved_tables = vec![None; lookups.len()];
    let mut missing_keys = Vec::new();
    let mut missing_positions = Vec::new();

    {
        let cache = lookup_table_cache().read().await;
        for (position, lookup) in lookups.iter().enumerate() {
            let Some(addresses) = cache.get(&lookup.account_key) else {
                missing_keys.push(lookup.account_key);
                missing_positions.push(position);
                continue;
            };

            if lookup_indexes_fit(addresses.as_ref(), lookup) {
                *resolved_tables.get_mut(position)? = Some(Arc::clone(addresses));
            } else {
                missing_keys.push(lookup.account_key);
                missing_positions.push(position);
            }
        }
    }

    if !missing_keys.is_empty() {
        let accounts = rpc.get_multiple_accounts(&missing_keys).await.ok()?;
        if accounts.len() != missing_keys.len() {
            return None;
        }

        let mut cache = lookup_table_cache().write().await;
        for ((position, lookup_key), account) in missing_positions
            .into_iter()
            .zip(missing_keys.into_iter())
            .zip(accounts.into_iter())
        {
            let account = account?;
            let table = AddressLookupTable::deserialize(&account.data).ok()?;
            let addresses = Arc::<[Pubkey]>::from(table.addresses.to_vec());
            cache.insert(lookup_key, Arc::clone(&addresses));
            *resolved_tables.get_mut(position)? = Some(addresses);
        }
    }

    let mut writable = Vec::new();
    let mut readonly = Vec::new();

    for (lookup, addresses) in lookups.iter().zip(resolved_tables.into_iter()) {
        let addresses = addresses?;
        for &index in &lookup.writable_indexes {
            writable.push(*addresses.get(usize::from(index))?);
        }

        for &index in &lookup.readonly_indexes {
            readonly.push(*addresses.get(usize::from(index))?);
        }
    }

    Some(LoadedAddresses { writable, readonly })
}

fn lookup_table_cache() -> &'static LookupTableCache {
    static CACHE: OnceLock<LookupTableCache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn lookup_indexes_fit(addresses: &[Pubkey], lookup: &MessageAddressTableLookup) -> bool {
    let max_index = lookup
        .writable_indexes
        .iter()
        .chain(lookup.readonly_indexes.iter())
        .copied()
        .max()
        .map(usize::from)
        .unwrap_or(0);
    max_index < addresses.len()
}

fn resolve_instruction_accounts(
    account_keys: &[Pubkey],
    instruction: &CompiledInstruction,
) -> Option<Vec<Pubkey>> {
    let mut resolved = Vec::with_capacity(instruction.accounts.len());
    for index in &instruction.accounts {
        resolved.push(*account_keys.get(usize::from(*index))?);
    }
    Some(resolved)
}

fn parse_cpmm_creation_data(data: &[u8]) -> Option<(u64, u64, u64)> {
    if !is_cpmm_creation_instruction(data) {
        return None;
    }

    Some((
        read_u64_le(data, STANDARD_AMM_INITIALIZE.len())?,
        read_u64_le(data, STANDARD_AMM_INITIALIZE.len().saturating_add(8))?,
        read_u64_le(data, STANDARD_AMM_INITIALIZE.len().saturating_add(16))?,
    ))
}

fn parse_openbook_creation_data(data: &[u8]) -> Option<(u64, u64, i64)> {
    match data.first().copied()? {
        RAYDIUM_V4_INITIALIZE2_TAG => Some((
            read_u64_le(data, 10)?,
            read_u64_le(data, 18)?,
            i64::try_from(read_u64_le(data, 2)?).ok()?,
        )),
        RAYDIUM_V4_INITIALIZE_TAG => None,
        _ => None,
    }
}

fn read_u64_le(data: &[u8], start: usize) -> Option<u64> {
    let end = start.checked_add(8)?;
    let bytes: [u8; 8] = data.get(start..end)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

const fn wsol_pubkey() -> Pubkey {
    Pubkey::from_str_const(super::constants::WSOL_ADDRESS)
}

#[cfg(test)]
mod tests {
    use solana_sdk::{message::compiled_instruction::CompiledInstruction, pubkey::Pubkey};

    use super::{
        RAYDIUM_V4_INITIALIZE2_TAG, RAYDIUM_V4_SWAP_BASE_IN_TAG, RaydiumStructuredCandidateKind,
        classify_raydium_creation_instructions, is_cpmm_creation_instruction,
        is_openbook_creation_instruction, parse_cpmm_creation_data, parse_openbook_creation_data,
        raydium_v4_program_pubkey,
    };
    use crate::adapters::raydium::{
        RAYDIUM_STANDARD_AMM_PROGRAM_ID, STANDARD_AMM_INITIALIZE, STANDARD_AMM_SWAP_BASE_INPUT,
        STANDARD_AMM_SWAP_BASE_OUTPUT,
    };

    #[test]
    fn cpmm_creation_whitelists_initialize_variants() {
        assert!(is_cpmm_creation_instruction(&STANDARD_AMM_INITIALIZE));
        assert!(!is_cpmm_creation_instruction(&STANDARD_AMM_SWAP_BASE_INPUT));
        assert!(!is_cpmm_creation_instruction(
            &STANDARD_AMM_SWAP_BASE_OUTPUT
        ));
    }

    #[test]
    fn openbook_creation_matches_supported_initialize_only() {
        assert!(is_openbook_creation_instruction(&[
            RAYDIUM_V4_INITIALIZE2_TAG
        ]));
        assert!(!is_openbook_creation_instruction(&[
            super::RAYDIUM_V4_INITIALIZE_TAG
        ]));
        assert!(!is_openbook_creation_instruction(&[
            RAYDIUM_V4_SWAP_BASE_IN_TAG
        ]));
        assert!(!is_openbook_creation_instruction(&[]));
    }

    #[test]
    fn structured_classifier_rejects_swap_traffic() {
        let cpmm_program = Pubkey::from_str_const(RAYDIUM_STANDARD_AMM_PROGRAM_ID);
        let openbook_program = raydium_v4_program_pubkey();
        let account_keys = vec![cpmm_program, openbook_program];
        let cpmm_swap = CompiledInstruction::new_from_raw_parts(
            0,
            STANDARD_AMM_SWAP_BASE_INPUT.to_vec(),
            vec![],
        );
        let openbook_swap =
            CompiledInstruction::new_from_raw_parts(1, vec![RAYDIUM_V4_SWAP_BASE_IN_TAG], vec![]);

        assert_eq!(
            classify_raydium_creation_instructions(
                &account_keys,
                &[cpmm_swap, openbook_swap],
                cpmm_program,
                openbook_program,
            ),
            None
        );
    }

    #[test]
    fn structured_classifier_detects_creation_instructions() {
        let cpmm_program = Pubkey::from_str_const(RAYDIUM_STANDARD_AMM_PROGRAM_ID);
        let openbook_program = raydium_v4_program_pubkey();
        let account_keys = vec![cpmm_program, openbook_program];
        let openbook_init =
            CompiledInstruction::new_from_raw_parts(1, vec![RAYDIUM_V4_INITIALIZE2_TAG], vec![]);

        assert_eq!(
            classify_raydium_creation_instructions(
                &account_keys,
                &[openbook_init],
                cpmm_program,
                openbook_program,
            ),
            Some(RaydiumStructuredCandidateKind::OpenBook)
        );
    }

    #[test]
    fn cpmm_creation_data_parses_amounts_and_open_time() {
        let mut data = STANDARD_AMM_INITIALIZE.to_vec();
        data.extend_from_slice(&11_u64.to_le_bytes());
        data.extend_from_slice(&22_u64.to_le_bytes());
        data.extend_from_slice(&33_u64.to_le_bytes());

        assert_eq!(parse_cpmm_creation_data(&data), Some((11, 22, 33)));
    }

    #[test]
    fn openbook_initialize2_data_parses_amounts_and_open_time() {
        let mut data = vec![RAYDIUM_V4_INITIALIZE2_TAG, 7];
        data.extend_from_slice(&44_u64.to_le_bytes());
        data.extend_from_slice(&55_u64.to_le_bytes());
        data.extend_from_slice(&66_u64.to_le_bytes());

        assert_eq!(parse_openbook_creation_data(&data), Some((55, 66, 44)));
    }
}
