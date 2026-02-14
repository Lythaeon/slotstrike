use std::{str::FromStr, sync::Arc};

use chrono::{Local, TimeZone};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSendTransactionConfig, RpcTransactionConfig},
};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Signature,
    signer::Signer,
    transaction::Transaction,
};
use solana_system_interface::instruction::transfer;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiInstruction, UiMessage,
    UiParsedInstruction, UiTransactionEncoding,
};
use spl_associated_token_account::{
    get_associated_token_address, get_associated_token_address_with_program_id,
    instruction::{create_associated_token_account, create_associated_token_account_idempotent},
};
use spl_token::instruction::{close_account, sync_native};

use crate::{
    MAX_RETRIES,
    adapters::raydium::{
        RAYDIUM_STANDARD_AMM_PROGRAM_ID, STANDARD_AMM_SWAP_BASE_INPUT, WSOL_ADDRESS, pool_open_time,
    },
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{IngressMetadata, unix_timestamp_now_ns},
        services::RuleMatcher,
        value_objects::{TxSubmissionMode, sol_amount::Lamports},
    },
    slices::sniper::{cache, log_parser::extract_u64_after_prefix},
};

#[derive(Clone, Copy)]
struct CpmmAccounts<'account_data> {
    deployer_address: &'account_data str,
    amm_config: &'account_data str,
    authority: &'account_data str,
    pool_state: &'account_data str,
    mint_a: &'account_data str,
    mint_b: &'account_data str,
    vault_a: &'account_data str,
    vault_b: &'account_data str,
    observation_state: &'account_data str,
    token_program_a: &'account_data str,
    token_program_b: &'account_data str,
}

impl<'account_data> CpmmAccounts<'account_data> {
    fn parse(accounts: &'account_data [String]) -> Option<Self> {
        Some(Self {
            deployer_address: accounts.first()?.as_str(),
            amm_config: accounts.get(1)?.as_str(),
            authority: accounts.get(2)?.as_str(),
            pool_state: accounts.get(3)?.as_str(),
            mint_a: accounts.get(4)?.as_str(),
            mint_b: accounts.get(5)?.as_str(),
            vault_a: accounts.get(10)?.as_str(),
            vault_b: accounts.get(11)?.as_str(),
            observation_state: accounts.get(13)?.as_str(),
            token_program_a: accounts.get(15)?.as_str(),
            token_program_b: accounts.get(16)?.as_str(),
        })
    }

    fn token_address(&self) -> Option<&'account_data str> {
        match (self.mint_a == WSOL_ADDRESS, self.mint_b == WSOL_ADDRESS) {
            (true, false) => Some(self.mint_b),
            (false, true) => Some(self.mint_a),
            _ => None,
        }
    }

    fn token_program(&self) -> Option<&'account_data str> {
        match (self.mint_a == WSOL_ADDRESS, self.mint_b == WSOL_ADDRESS) {
            (true, false) => Some(self.token_program_b),
            (false, true) => Some(self.token_program_a),
            _ => None,
        }
    }

    fn token_is_vault_zero(&self) -> bool {
        self.mint_a != WSOL_ADDRESS
    }

    fn input_vault(&self) -> &'account_data str {
        if self.token_is_vault_zero() {
            self.vault_b
        } else {
            self.vault_a
        }
    }

    fn output_vault(&self) -> &'account_data str {
        if self.token_is_vault_zero() {
            self.vault_a
        } else {
            self.vault_b
        }
    }
}

pub async fn handle_cpmm_event(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    logs: Vec<String>,
    signature: Signature,
    ingress_metadata: IngressMetadata,
) {
    let vault_0_amount = match extract_u64_after_prefix(&logs, "vault_0_amount:") {
        Some(value) => value,
        None => return,
    };

    let vault_1_amount = match extract_u64_after_prefix(&logs, "vault_1_amount:") {
        Some(value) => value,
        None => return,
    };

    log::debug!(
        "CPMM > vault_0_amount: {}, vault_1_amount: {}",
        vault_0_amount,
        vault_1_amount
    );

    let ingress_latency_ns =
        unix_timestamp_now_ns().saturating_sub(ingress_metadata.normalized_timestamp_ns);
    log::debug!(
        "CPMM > ingress source={}, normalized_ts={}ns, hw_ts={:?}, latency={}ns",
        ingress_metadata.source.as_str(),
        ingress_metadata.normalized_timestamp_ns,
        ingress_metadata.hardware_timestamp_ns,
        ingress_latency_ns
    );

    let creation_tx =
        match fetch_transaction_with_retries(context.rpc.as_ref(), &signature, "CPMM").await {
            Some(value) => value,
            None => return,
        };

    log::debug!("CPMM > Pool creation transaction: {:?}", creation_tx);

    let accounts = match extract_program_accounts(&creation_tx, RAYDIUM_STANDARD_AMM_PROGRAM_ID) {
        Some(value) => value,
        None => return,
    };

    let parsed_accounts = match CpmmAccounts::parse(&accounts) {
        Some(value) => value,
        None => return,
    };

    log::debug!("CPMM > Pool creation accounts: {:?}", accounts);

    let token_address = match parsed_accounts.token_address() {
        Some(value) => value,
        None => return,
    };
    let token_program = match parsed_accounts.token_program() {
        Some(value) => value,
        None => return,
    };
    let deployer_address = parsed_accounts.deployer_address;

    let matched_rule =
        match RuleMatcher::match_rule(rulebook.as_ref(), token_address, deployer_address) {
            Some(value) => value,
            None => {
                log::debug!("CPMM > {} > Ignoring token", token_address);
                return;
            }
        };

    log::debug!(
        "CPMM > {} > Matched by {:?} rule key {}",
        token_address,
        matched_rule.source,
        matched_rule.cold.address
    );

    log::debug!(
        "CPMM > {} > Snipe height: {} SOL, Jito tip: {} SOL, Slippage: {} %",
        token_address,
        matched_rule.hot.snipe_height().as_sol_string(),
        matched_rule.hot.jito_tip().as_sol_string(),
        matched_rule.hot.slippage().as_pct_string(),
    );

    log::info!("CPMM > Found token: {}", token_address);
    log::info!("CPMM > {} > Creating transaction", token_address);

    let program_id = match cache::raydium_standard_amm_program_pubkey() {
        Some(value) => value,
        None => return,
    };

    let authority = match Pubkey::from_str(parsed_accounts.authority) {
        Ok(value) => value,
        Err(_) => return,
    };

    let amm_config = match Pubkey::from_str(parsed_accounts.amm_config) {
        Ok(value) => value,
        Err(_) => return,
    };

    let pool_state = match Pubkey::from_str(parsed_accounts.pool_state) {
        Ok(value) => value,
        Err(_) => return,
    };

    let input_vault = match Pubkey::from_str(parsed_accounts.input_vault()) {
        Ok(value) => value,
        Err(_) => return,
    };

    let output_vault = match Pubkey::from_str(parsed_accounts.output_vault()) {
        Ok(value) => value,
        Err(_) => return,
    };

    let observation_state = match Pubkey::from_str(parsed_accounts.observation_state) {
        Ok(value) => value,
        Err(_) => return,
    };

    log::debug!(
        "CPMM > {} > Authority: {}, AMM config: {}, Pool state: {}, Input vault: {}, Output vault: {}, Observation state: {}",
        token_address,
        authority,
        amm_config,
        pool_state,
        input_vault,
        output_vault,
        observation_state,
    );

    log::info!("CPMM > {} > Requesting pool data", token_address);

    let pool = match fetch_pool_with_retries(context.rpc.as_ref(), &pool_state, token_address).await
    {
        Some(value) => value,
        None => return,
    };

    log::debug!("CPMM > {} > Pool response: {:?}", token_address, pool);

    let pool_value = match pool.value {
        Some(value) => value,
        None => return,
    };

    let pool_open_timestamp = match pool_open_time(&pool_value.data) {
        Some(value) => value,
        None => return,
    };
    log::debug!(
        "CPMM > {} > Pool open time: {}",
        token_address,
        pool_open_timestamp
    );

    let lamports = matched_rule.hot.snipe_height().as_lamports().as_u64();
    let wsol_pubkey = match cache::wsol_pubkey() {
        Some(value) => value,
        None => return,
    };

    let token_pubkey = match Pubkey::from_str(token_address) {
        Ok(value) => value,
        Err(_) => return,
    };

    let token_program_pubkey = match Pubkey::from_str(token_program) {
        Ok(value) => value,
        Err(_) => return,
    };

    let user_in_token_account =
        get_associated_token_address(&context.keypair.pubkey(), &wsol_pubkey);
    let user_out_token_account = get_associated_token_address_with_program_id(
        &context.keypair.pubkey(),
        &token_pubkey,
        &token_program_pubkey,
    );

    let token_program_id = match cache::token_program_pubkey() {
        Some(value) => value,
        None => return,
    };

    let mut instructions = Vec::with_capacity(9);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(120_000));
    instructions.push(ComputeBudgetInstruction::set_compute_unit_price(
        context.priority_fees,
    ));

    instructions.push(create_associated_token_account_idempotent(
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &wsol_pubkey,
        &token_program_id,
    ));

    instructions.push(transfer(
        &context.keypair.pubkey(),
        &user_in_token_account,
        lamports,
    ));

    let sync_instruction = match sync_native(&spl_token::ID, &user_in_token_account) {
        Ok(value) => value,
        Err(error) => {
            log::error!("CPMM > {} > sync_native failed: {}", token_address, error);
            return;
        }
    };
    instructions.push(sync_instruction);

    instructions.push(create_associated_token_account(
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &token_pubkey,
        &token_program_pubkey,
    ));

    let min_amount_out = calculate_min_amount_out(
        lamports,
        matched_rule.hot.slippage().as_bps(),
        vault_0_amount,
        vault_1_amount,
        parsed_accounts.token_is_vault_zero(),
    );

    log::debug!(
        "CPMM > {} > Min amount out: {}",
        token_address,
        min_amount_out
    );

    let mut swap_data = Vec::with_capacity(24);
    swap_data.extend_from_slice(&STANDARD_AMM_SWAP_BASE_INPUT);
    swap_data.extend_from_slice(&lamports.to_le_bytes());
    swap_data.extend_from_slice(&min_amount_out.to_le_bytes());

    instructions.push(Instruction::new_with_bytes(
        program_id,
        &swap_data,
        vec![
            AccountMeta::new_readonly(context.keypair.pubkey(), true),
            AccountMeta::new_readonly(authority, false),
            AccountMeta::new_readonly(amm_config, false),
            AccountMeta::new(pool_state, false),
            AccountMeta::new(user_in_token_account, false),
            AccountMeta::new(user_out_token_account, false),
            AccountMeta::new(input_vault, false),
            AccountMeta::new(output_vault, false),
            AccountMeta::new_readonly(token_program_id, false),
            AccountMeta::new_readonly(token_program_pubkey, false),
            AccountMeta::new_readonly(wsol_pubkey, false),
            AccountMeta::new_readonly(token_pubkey, false),
            AccountMeta::new(observation_state, false),
        ],
    ));

    let close_instruction = match close_account(
        &token_program_id,
        &user_in_token_account,
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &[&context.keypair.pubkey()],
    ) {
        Ok(value) => value,
        Err(error) => {
            log::error!("CPMM > {} > close_account failed: {}", token_address, error);
            return;
        }
    };
    instructions.push(close_instruction);

    let jito_tip_lamports = matched_rule.hot.jito_tip().as_lamports().as_u64();
    if context.tx_submission_mode == TxSubmissionMode::Jito {
        let jito_tip_account = match cache::jito_tip_pubkey() {
            Some(value) => value,
            None => return,
        };

        instructions.push(transfer(
            &context.keypair.pubkey(),
            &jito_tip_account,
            jito_tip_lamports,
        ));
    }

    maybe_wait_for_pool_open(pool_open_timestamp, token_address, "CPMM").await;

    let blockhash = match context.rpc.get_latest_blockhash().await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "CPMM > {} > Failed to fetch blockhash: {}",
                token_address,
                error
            );
            return;
        }
    };

    let swap_tx = {
        let signer_refs: [&dyn Signer; 1] = [context.keypair.as_ref()];
        Transaction::new_signed_with_payer(
            &instructions,
            Some(&context.keypair.pubkey()),
            &signer_refs,
            blockhash,
        )
    };

    log::info!("CPMM > {} > Starting swap", token_address);

    let sent_signature = match submit_swap_transaction(context.as_ref(), &swap_tx).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "CPMM > {} > Failed to send transaction: {}",
                token_address,
                error
            );
            return;
        }
    };

    log::info!(
        "CPMM > {} > Swap transaction signature: {}",
        token_address,
        sent_signature
    );

    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

    let status = match context.rpc.get_signature_status(&sent_signature).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "CPMM > {} > Signature status failed: {}",
                token_address,
                error
            );
            return;
        }
    };

    let Some(status) = status else {
        log::error!("CPMM > {} > No signature status returned", token_address);
        return;
    };

    if let Err(error) = status {
        log::error!(
            "CPMM > {} > Swap transaction failed: {}",
            token_address,
            error
        );
        return;
    }

    let balance = match context.rpc.get_balance(&context.keypair.pubkey()).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "CPMM > {} > Failed to fetch balance: {}",
                token_address,
                error
            );
            return;
        }
    };

    log::info!(
        "CPMM > {} > Successfully swapped {} SOL with {} SOL tip budget (mode={})",
        token_address,
        matched_rule.hot.snipe_height().as_sol_string(),
        matched_rule.hot.jito_tip().as_sol_string(),
        context.tx_submission_mode.as_str(),
    );
    log::info!(
        "CPMM > {} > Balance: {} SOL",
        token_address,
        Lamports::new(balance).as_sol_string()
    );
}

async fn submit_swap_transaction(
    context: &ExecutionContext,
    swap_tx: &Transaction,
) -> Result<Signature, solana_client::client_error::ClientError> {
    let send_config = RpcSendTransactionConfig {
        skip_preflight: true,
        encoding: Some(UiTransactionEncoding::Base58),
        max_retries: Some(0),
        ..RpcSendTransactionConfig::default()
    };

    if context.tx_submission_mode == TxSubmissionMode::Direct {
        return context
            .rpc
            .send_transaction_with_config(swap_tx, send_config)
            .await;
    }

    let jito_rpc = RpcClient::new(context.jito_url.as_ref().clone());
    jito_rpc
        .send_transaction_with_config(swap_tx, send_config)
        .await
}

async fn fetch_transaction_with_retries(
    rpc: &RpcClient,
    signature: &Signature,
    label: &str,
) -> Option<EncodedConfirmedTransactionWithStatusMeta> {
    let mut retries = 0_usize;

    loop {
        match rpc
            .get_transaction_with_config(
                signature,
                RpcTransactionConfig {
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: Some(0),
                    encoding: Some(UiTransactionEncoding::JsonParsed),
                },
            )
            .await
        {
            Ok(tx) => return Some(tx),
            Err(error) => {
                if !error.to_string().contains("invalid type: null") {
                    log::error!("{} > Error getting transaction: {}", label, error);
                } else {
                    log::debug!("{} > Error getting transaction: {}", label, error);
                }

                retries = retries.saturating_add(1);
                if retries >= MAX_RETRIES {
                    log::error!(
                        "{} > Max retries reached in transaction. Exiting loop.",
                        label
                    );
                    return None;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
            }
        }
    }
}

async fn fetch_pool_with_retries(
    rpc: &RpcClient,
    pool_state: &Pubkey,
    token_address: &str,
) -> Option<solana_client::rpc_response::Response<Option<solana_sdk::account::Account>>> {
    let mut retries = 0_usize;

    loop {
        match rpc
            .get_account_with_commitment(pool_state, CommitmentConfig::confirmed())
            .await
        {
            Ok(pool) => {
                if pool.value.is_none() {
                    log::debug!(
                        "CPMM > {} > Pool not available yet: {}",
                        token_address,
                        pool_state
                    );
                    retries = retries.saturating_add(1);
                    if retries >= MAX_RETRIES {
                        log::error!("CPMM > Max retries reached in pool. Exiting loop.");
                        return None;
                    }

                    tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
                    continue;
                }

                return Some(pool);
            }
            Err(error) => {
                if !error.to_string().contains("invalid type: null") {
                    log::error!("CPMM > Error getting pool data: {}", error);
                } else {
                    log::debug!("CPMM > Error getting pool data: {}", error);
                }

                retries = retries.saturating_add(1);
                if retries >= MAX_RETRIES {
                    log::error!("CPMM > Max retries reached in pool. Exiting loop.");
                    return None;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
            }
        }
    }
}

fn extract_program_accounts(
    tx: &EncodedConfirmedTransactionWithStatusMeta,
    program_id: &str,
) -> Option<Vec<String>> {
    let EncodedTransaction::Json(json_tx) = &tx.transaction.transaction else {
        return None;
    };

    let UiMessage::Parsed(message) = &json_tx.message else {
        return None;
    };

    for instruction in &message.instructions {
        let UiInstruction::Parsed(parsed_instruction) = instruction else {
            continue;
        };

        let UiParsedInstruction::PartiallyDecoded(decoded_instruction) = parsed_instruction else {
            continue;
        };

        if decoded_instruction.program_id == program_id {
            return Some(decoded_instruction.accounts.clone());
        }
    }

    None
}

#[inline(always)]
fn calculate_min_amount_out(
    lamports: u64,
    slippage_bps: u16,
    vault_0_amount: u64,
    vault_1_amount: u64,
    token_is_vault_zero: bool,
) -> u64 {
    if vault_0_amount == 0 || vault_1_amount == 0 {
        return 0;
    }

    if slippage_bps >= 10_000 {
        return 0;
    }

    let lamports_u128 = u128::from(lamports);
    let max_amount_out = if token_is_vault_zero {
        lamports_u128
            .checked_mul(u128::from(vault_0_amount))
            .and_then(|value| value.checked_div(u128::from(vault_1_amount)))
    } else {
        lamports_u128
            .checked_mul(u128::from(vault_1_amount))
            .and_then(|value| value.checked_div(u128::from(vault_0_amount)))
    };

    let remaining_bps = 10_000_u128.saturating_sub(u128::from(slippage_bps));
    let min_amount_out = max_amount_out
        .and_then(|value| value.checked_mul(remaining_bps))
        .and_then(|value| value.checked_div(10_000_u128));

    min_amount_out
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(u64::MAX)
}

async fn maybe_wait_for_pool_open(pool_open_time: u64, token_address: &str, label: &str) {
    let now = Local::now();

    let Some(target_time) = Local.timestamp_opt(pool_open_time as i64, 0).single() else {
        return;
    };

    if now >= target_time {
        return;
    }

    let duration = target_time.signed_duration_since(now);
    let remaining_minutes = duration.num_minutes();
    let remaining_seconds = duration
        .num_seconds()
        .saturating_sub(remaining_minutes.saturating_mul(60));

    log::info!(
        "{} > {} > Pool closed. Proceeding with snipe in {}m {}s. UTC: {}",
        label,
        token_address,
        remaining_minutes,
        remaining_seconds,
        target_time.to_rfc2822(),
    );

    if let Ok(duration) = duration.to_std() {
        tokio::time::sleep(duration).await;
    }
}

#[cfg(test)]
mod tests {
    use super::calculate_min_amount_out;

    #[test]
    fn min_amount_out_uses_integer_fixed_point_math() {
        let min = calculate_min_amount_out(1_000, 100, 5_000, 10_000, true);
        assert_eq!(min, 495);
    }

    #[test]
    fn min_amount_out_returns_zero_for_invalid_bounds() {
        assert_eq!(calculate_min_amount_out(1_000, 0, 0, 10_000, true), 0);
        assert_eq!(
            calculate_min_amount_out(1_000, 10_000, 5_000, 10_000, true),
            0
        );
    }

    #[test]
    fn min_amount_out_saturates_on_internal_overflow() {
        let min = calculate_min_amount_out(u64::MAX, 1, u64::MAX, 1, true);
        assert_eq!(min, u64::MAX);
    }
}
