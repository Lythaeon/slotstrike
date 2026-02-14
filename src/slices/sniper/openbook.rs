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
    get_associated_token_address,
    instruction::{create_associated_token_account, create_associated_token_account_idempotent},
};
use spl_token::instruction::{close_account, sync_native};

use crate::{
    MAX_RETRIES,
    adapters::raydium::{
        RAYDIUM_V4_PROGRAM_ID, SwapInstructionBaseIn, WSOL_ADDRESS, get_associated_authority,
        get_market_accounts,
    },
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{IngressMetadata, unix_timestamp_now_ns},
        services::RuleMatcher,
        value_objects::{TxSubmissionMode, sol_amount::Lamports},
    },
    slices::sniper::{
        cache,
        log_parser::{extract_i64_after_prefix, extract_u64_after_prefix},
    },
};

#[derive(Clone, Copy)]
struct OpenbookAccounts<'account_data> {
    id: &'account_data str,
    authority: &'account_data str,
    open_orders: &'account_data str,
    mint_a: &'account_data str,
    mint_b: &'account_data str,
    base_vault: &'account_data str,
    quote_vault: &'account_data str,
    target_orders: &'account_data str,
    market_program_id: &'account_data str,
    market_id: &'account_data str,
    deployer_address: &'account_data str,
}

impl<'account_data> OpenbookAccounts<'account_data> {
    fn parse(accounts: &'account_data [String]) -> Option<Self> {
        Some(Self {
            id: accounts.get(4)?.as_str(),
            authority: accounts.get(5)?.as_str(),
            open_orders: accounts.get(6)?.as_str(),
            mint_a: accounts.get(8)?.as_str(),
            mint_b: accounts.get(9)?.as_str(),
            base_vault: accounts.get(10)?.as_str(),
            quote_vault: accounts.get(11)?.as_str(),
            target_orders: accounts.get(12)?.as_str(),
            market_program_id: accounts.get(15)?.as_str(),
            market_id: accounts.get(16)?.as_str(),
            deployer_address: accounts.get(17)?.as_str(),
        })
    }

    fn token_address(&self) -> Option<&'account_data str> {
        match (self.mint_a == WSOL_ADDRESS, self.mint_b == WSOL_ADDRESS) {
            (true, false) => Some(self.mint_b),
            (false, true) => Some(self.mint_a),
            _ => None,
        }
    }

    fn token_is_coin_mint(&self) -> bool {
        self.mint_a != WSOL_ADDRESS
    }
}

pub async fn handle_openbook_event(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    logs: Vec<String>,
    signature: Signature,
    ingress_metadata: IngressMetadata,
) {
    let init_pc_amount = match extract_u64_after_prefix(&logs, "init_pc_amount: ") {
        Some(value) => value,
        None => return,
    };

    let init_coin_amount = match extract_u64_after_prefix(&logs, "init_coin_amount: ") {
        Some(value) => value,
        None => return,
    };

    let open_timestamp = match extract_i64_after_prefix(&logs, "open_time: ") {
        Some(value) => value,
        None => return,
    };

    log::debug!(
        "OpenBook > init_pc_amount: {}, init_coin_amount: {}, open_time: {}",
        init_pc_amount,
        init_coin_amount,
        open_timestamp
    );

    let ingress_latency_ns =
        unix_timestamp_now_ns().saturating_sub(ingress_metadata.normalized_timestamp_ns);
    log::debug!(
        "OpenBook > ingress source={}, normalized_ts={}ns, hw_ts={:?}, latency={}ns",
        ingress_metadata.source.as_str(),
        ingress_metadata.normalized_timestamp_ns,
        ingress_metadata.hardware_timestamp_ns,
        ingress_latency_ns
    );

    let creation_tx =
        match fetch_transaction_with_retries(context.rpc.as_ref(), &signature, "OpenBook").await {
            Some(value) => value,
            None => return,
        };

    log::debug!("OpenBook > Pool creation transaction: {:?}", creation_tx);

    let accounts = match extract_program_accounts(&creation_tx, RAYDIUM_V4_PROGRAM_ID) {
        Some(value) => value,
        None => return,
    };
    let parsed_accounts = match OpenbookAccounts::parse(&accounts) {
        Some(value) => value,
        None => return,
    };

    log::debug!("OpenBook > Pool creation accounts: {:?}", accounts);

    let token_address = match parsed_accounts.token_address() {
        Some(value) => value,
        None => return,
    };

    let deployer_address = parsed_accounts.deployer_address;

    let matched_rule =
        match RuleMatcher::match_rule(rulebook.as_ref(), token_address, deployer_address) {
            Some(value) => value,
            None => {
                log::debug!("OpenBook > {} > Ignoring token", token_address);
                return;
            }
        };

    log::debug!(
        "OpenBook > {} > Matched by {:?} rule key {}",
        token_address,
        matched_rule.source,
        matched_rule.cold.address
    );

    log::debug!(
        "OpenBook > {} > Snipe height: {} SOL, Jito tip: {} SOL, Slippage: {} %",
        token_address,
        matched_rule.hot.snipe_height().as_sol_string(),
        matched_rule.hot.jito_tip().as_sol_string(),
        matched_rule.hot.slippage().as_pct_string()
    );

    log::info!("OpenBook > {} > Found token", token_address);

    let id = match Pubkey::from_str(parsed_accounts.id) {
        Ok(value) => value,
        Err(_) => return,
    };
    let authority = match Pubkey::from_str(parsed_accounts.authority) {
        Ok(value) => value,
        Err(_) => return,
    };
    let open_orders = match Pubkey::from_str(parsed_accounts.open_orders) {
        Ok(value) => value,
        Err(_) => return,
    };
    let base_vault = match Pubkey::from_str(parsed_accounts.base_vault) {
        Ok(value) => value,
        Err(_) => return,
    };
    let quote_vault = match Pubkey::from_str(parsed_accounts.quote_vault) {
        Ok(value) => value,
        Err(_) => return,
    };
    let target_orders = match Pubkey::from_str(parsed_accounts.target_orders) {
        Ok(value) => value,
        Err(_) => return,
    };
    let market_program_id = match Pubkey::from_str(parsed_accounts.market_program_id) {
        Ok(value) => value,
        Err(_) => return,
    };
    let market_id = match Pubkey::from_str(parsed_accounts.market_id) {
        Ok(value) => value,
        Err(_) => return,
    };

    log::debug!(
        "OpenBook > {} > ID: {}, Authority: {}, Open orders: {}, Base vault: {}, Quote vault: {}, Target orders: {}, Market program ID: {}, Market ID: {}",
        token_address,
        id,
        authority,
        open_orders,
        base_vault,
        quote_vault,
        target_orders,
        market_program_id,
        market_id,
    );

    let market = match get_market_accounts(&context.rpc, &market_id).await {
        Some(value) => value,
        None => return,
    };

    let lamports = matched_rule.hot.snipe_height().as_lamports().as_u64();

    let wsol_pubkey = match cache::wsol_pubkey() {
        Some(value) => value,
        None => return,
    };

    let token_pubkey = match Pubkey::from_str(token_address) {
        Ok(value) => value,
        Err(_) => return,
    };

    let token_program_id = match cache::token_program_pubkey() {
        Some(value) => value,
        None => return,
    };

    let user_in_token_account =
        get_associated_token_address(&context.keypair.pubkey(), &wsol_pubkey);
    let user_out_token_account =
        get_associated_token_address(&context.keypair.pubkey(), &token_pubkey);

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
            log::error!(
                "OpenBook > {} > sync_native failed: {}",
                token_address,
                error
            );
            return;
        }
    };
    instructions.push(sync_instruction);

    instructions.push(create_associated_token_account(
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &token_pubkey,
        &token_program_id,
    ));

    let min_amount_out = calculate_min_amount_out(
        lamports,
        matched_rule.hot.slippage().as_bps(),
        init_pc_amount,
        init_coin_amount,
        parsed_accounts.token_is_coin_mint(),
    );

    log::debug!(
        "OpenBook > {} > Min amount out: {}",
        token_address,
        min_amount_out
    );

    let market_authority =
        match get_associated_authority(&market.program_id, &market.state.own_address) {
            Ok(value) => value.0,
            Err(_) => return,
        };

    let swap_instruction = Instruction::new_with_borsh(
        match cache::raydium_v4_program_pubkey() {
            Some(value) => value,
            None => return,
        },
        &SwapInstructionBaseIn {
            discriminator: 9,
            amount_in: lamports,
            minimum_amount_out: min_amount_out,
        },
        vec![
            AccountMeta::new_readonly(token_program_id, false),
            AccountMeta::new(id, false),
            AccountMeta::new_readonly(authority, false),
            AccountMeta::new(open_orders, false),
            AccountMeta::new(target_orders, false),
            AccountMeta::new(base_vault, false),
            AccountMeta::new(quote_vault, false),
            AccountMeta::new_readonly(market_program_id, false),
            AccountMeta::new(market_id, false),
            AccountMeta::new(market.state.bids, false),
            AccountMeta::new(market.state.asks, false),
            AccountMeta::new(market.state.event_queue, false),
            AccountMeta::new(market.state.base_vault, false),
            AccountMeta::new(market.state.quote_vault, false),
            AccountMeta::new_readonly(market_authority, false),
            AccountMeta::new(user_in_token_account, false),
            AccountMeta::new(user_out_token_account, false),
            AccountMeta::new_readonly(context.keypair.pubkey(), true),
        ],
    );
    instructions.push(swap_instruction);

    let close_instruction = match close_account(
        &token_program_id,
        &user_in_token_account,
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &[&context.keypair.pubkey()],
    ) {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "OpenBook > {} > close_account failed: {}",
                token_address,
                error
            );
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

    maybe_wait_for_pool_open(open_timestamp, token_address, "OpenBook").await;

    let blockhash = match context.rpc.get_latest_blockhash().await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "OpenBook > {} > Failed to fetch blockhash: {}",
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

    let sent_signature = match submit_swap_transaction(context.as_ref(), &swap_tx).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "OpenBook > {} > Failed to send transaction: {}",
                token_address,
                error
            );
            return;
        }
    };

    log::info!(
        "OpenBook > {} > Swap transaction signature: {}",
        token_address,
        sent_signature
    );

    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

    let status = match context.rpc.get_signature_status(&sent_signature).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "OpenBook > {} > Signature status failed: {}",
                token_address,
                error
            );
            return;
        }
    };

    let Some(status) = status else {
        log::error!(
            "OpenBook > {} > No signature status returned",
            token_address
        );
        return;
    };

    if let Err(error) = status {
        log::error!(
            "OpenBook > {} > Swap transaction failed: {}",
            token_address,
            error
        );
        return;
    }

    let balance = match context.rpc.get_balance(&context.keypair.pubkey()).await {
        Ok(value) => value,
        Err(error) => {
            log::error!(
                "OpenBook > {} > Failed to fetch balance: {}",
                token_address,
                error
            );
            return;
        }
    };

    log::info!(
        "OpenBook > {} > Successfully swapped {} SOL with {} SOL tip budget (mode={})",
        token_address,
        matched_rule.hot.snipe_height().as_sol_string(),
        matched_rule.hot.jito_tip().as_sol_string(),
        context.tx_submission_mode.as_str(),
    );
    log::info!(
        "OpenBook > {} > Balance: {} SOL",
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
    init_pc_amount: u64,
    init_coin_amount: u64,
    token_is_coin_mint: bool,
) -> u64 {
    if init_pc_amount == 0 || init_coin_amount == 0 {
        return 0;
    }

    if slippage_bps >= 10_000 {
        return 0;
    }

    let lamports_u128 = u128::from(lamports);
    let max_amount_out = if token_is_coin_mint {
        lamports_u128
            .checked_mul(u128::from(init_coin_amount))
            .and_then(|value| value.checked_div(u128::from(init_pc_amount)))
    } else {
        lamports_u128
            .checked_mul(u128::from(init_pc_amount))
            .and_then(|value| value.checked_div(u128::from(init_coin_amount)))
    };

    let remaining_bps = 10_000_u128.saturating_sub(u128::from(slippage_bps));
    let min_amount_out = max_amount_out
        .and_then(|value| value.checked_mul(remaining_bps))
        .and_then(|value| value.checked_div(10_000_u128));

    min_amount_out
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(u64::MAX)
}

async fn maybe_wait_for_pool_open(open_timestamp: i64, token_address: &str, label: &str) {
    let now = Local::now();
    let Some(target_time) = Local.timestamp_opt(open_timestamp, 0).single() else {
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
        let min = calculate_min_amount_out(1_000, 100, 20_000, 10_000, true);
        assert_eq!(min, 495);
    }

    #[test]
    fn min_amount_out_returns_zero_for_invalid_bounds() {
        assert_eq!(calculate_min_amount_out(1_000, 0, 0, 10_000, true), 0);
        assert_eq!(
            calculate_min_amount_out(1_000, 10_000, 20_000, 10_000, true),
            0
        );
    }

    #[test]
    fn min_amount_out_saturates_on_internal_overflow() {
        let min = calculate_min_amount_out(u64::MAX, 1, u64::MAX, 1, false);
        assert_eq!(min, u64::MAX);
    }
}
