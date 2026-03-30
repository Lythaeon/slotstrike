use std::sync::Arc;

use chrono::{Local, TimeZone};
use sof_solana_compat::TxBuilder;
use sof_tx::SignedTx;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signature::Signature,
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction::transfer;
use solana_transaction_status::UiTransactionEncoding;
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use spl_token::instruction::{close_account, sync_native};

use crate::{
    adapters::raydium::{
        ParsedOpenbookCreation, SwapInstructionBaseIn, get_associated_authority,
        get_market_accounts, parse_openbook_creation_transaction,
    },
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{IngressMetadata, unix_timestamp_now_ns},
        services::RuleMatcher,
        value_objects::{TxSubmissionMode, sol_amount::Lamports},
    },
    slices::sniper::cache,
};

pub async fn handle_openbook_candidate_structured(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    transaction: Arc<solana_sdk::transaction::VersionedTransaction>,
    ingress_metadata: IngressMetadata,
) {
    let program_id = match cache::raydium_v4_program_pubkey() {
        Some(value) => value,
        None => return,
    };
    let creation = match parse_openbook_creation_transaction(
        context.rpc.as_ref(),
        transaction.as_ref(),
        program_id,
    )
    .await
    {
        Some(value) => value,
        None => return,
    };

    handle_openbook_transaction(context, rulebook, ingress_metadata, creation).await;
}

async fn handle_openbook_transaction(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    ingress_metadata: IngressMetadata,
    creation: ParsedOpenbookCreation,
) {
    log::debug!(
        "OpenBook > init_pc_amount: {}, init_coin_amount: {}, open_time: {}",
        creation.init_pc_amount,
        creation.init_coin_amount,
        creation.open_time
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

    let token_address = match creation.token_mint() {
        Some(value) => value,
        None => return,
    };
    let token_address_text = token_address.to_string();
    let deployer_address_text = creation.deployer_address.to_string();

    let matched_rule = match RuleMatcher::match_rule(
        rulebook.as_ref(),
        token_address_text.as_str(),
        deployer_address_text.as_str(),
    ) {
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

    log::debug!(
        "OpenBook > {} > ID: {}, Authority: {}, Open orders: {}, Base vault: {}, Quote vault: {}, Target orders: {}, Market program ID: {}, Market ID: {}",
        token_address,
        creation.id,
        creation.authority,
        creation.open_orders,
        creation.base_vault,
        creation.quote_vault,
        creation.target_orders,
        creation.market_program_id,
        creation.market_id,
    );

    let market = match get_market_accounts(&context.rpc, &creation.market_id).await {
        Some(value) => value,
        None => return,
    };

    let lamports = matched_rule.hot.snipe_height().as_lamports().as_u64();

    let wsol_pubkey = match cache::wsol_pubkey() {
        Some(value) => value,
        None => return,
    };

    let token_program_id = match cache::token_program_pubkey() {
        Some(value) => value,
        None => return,
    };

    let user_in_token_account =
        get_associated_token_address(&context.keypair.pubkey(), &wsol_pubkey);
    let user_out_token_account =
        get_associated_token_address(&context.keypair.pubkey(), &token_address);

    let mut instructions = Vec::with_capacity(7);

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

    instructions.push(create_associated_token_account_idempotent(
        &context.keypair.pubkey(),
        &context.keypair.pubkey(),
        &token_address,
        &token_program_id,
    ));

    let min_amount_out = calculate_min_amount_out(
        lamports,
        matched_rule.hot.slippage().as_bps(),
        creation.init_pc_amount,
        creation.init_coin_amount,
        creation.token_is_coin_mint(),
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
            AccountMeta::new(creation.id, false),
            AccountMeta::new_readonly(creation.authority, false),
            AccountMeta::new(creation.open_orders, false),
            AccountMeta::new(creation.target_orders, false),
            AccountMeta::new(creation.base_vault, false),
            AccountMeta::new(creation.quote_vault, false),
            AccountMeta::new_readonly(creation.market_program_id, false),
            AccountMeta::new(creation.market_id, false),
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
    if context.sof_tx_uses_jito || context.tx_submission_mode == TxSubmissionMode::Jito {
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

    maybe_wait_for_pool_open(creation.open_time, token_address_text.as_str(), "OpenBook").await;

    let blockhash = match context.latest_swap_blockhash().await {
        Ok(value) => value,
        Err(error) => {
            log::error!("OpenBook > {} > {}", token_address, error);
            return;
        }
    };

    let swap_tx = match build_swap_transaction(context.as_ref(), instructions, blockhash) {
        Ok(value) => value,
        Err(error) => {
            log::error!("OpenBook > {} > {}", token_address, error);
            return;
        }
    };

    let swap_signature = swap_tx.signatures.first().copied().unwrap_or_default();

    if context.dry_run {
        log::info!(
            "OpenBook > {} > Dry run built swap transaction: {} (submission skipped)",
            token_address,
            swap_signature
        );
        return;
    }

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

    match wait_for_signature_status(
        context.rpc.as_ref(),
        &sent_signature,
        token_address_text.as_str(),
        "OpenBook",
    )
    .await
    {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            log::error!(
                "OpenBook > {} > Swap transaction failed: {}",
                token_address,
                error
            );
            return;
        }
        None => return,
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

async fn wait_for_signature_status(
    rpc: &RpcClient,
    signature: &Signature,
    token_address: &str,
    label: &str,
) -> Option<Result<(), String>> {
    const MAX_CONFIRMATION_POLLS: usize = 120;
    let mut delay = tokio::time::Duration::from_millis(250);

    for _ in 0..MAX_CONFIRMATION_POLLS {
        let status = match rpc.get_signature_status(signature).await {
            Ok(value) => value,
            Err(error) => {
                log::error!(
                    "{} > {} > Signature status failed: {}",
                    label,
                    token_address,
                    error
                );
                return None;
            }
        };

        if let Some(status) = status {
            return Some(status.map_err(|error| error.to_string()));
        }

        tokio::time::sleep(delay).await;
        if delay < tokio::time::Duration::from_secs(2) {
            delay = delay
                .saturating_mul(2)
                .min(tokio::time::Duration::from_secs(2));
        }
    }

    log::error!(
        "{} > {} > No signature status returned before timeout",
        label,
        token_address
    );
    None
}

async fn submit_swap_transaction(
    context: &ExecutionContext,
    swap_tx: &VersionedTransaction,
) -> Result<Signature, String> {
    if let (Some(client), Some(plan)) = (&context.sof_tx_client, &context.sof_tx_plan) {
        let tx_bytes = bincode::serialize(swap_tx)
            .map_err(|error| format!("failed to serialize transaction for SOF-TX: {error}"))?;
        let mut client = client.lock().await;
        client
            .submit_signed_via(SignedTx::VersionedTransactionBytes(tx_bytes), plan.clone())
            .await
            .map_err(|error| format!("SOF-TX submit failed: {error}"))?;
        return Ok(swap_tx.signatures.first().copied().unwrap_or_default());
    }

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
            .await
            .map_err(|error| error.to_string());
    }

    let jito_rpc = RpcClient::new(context.jito_url.as_ref().clone());
    jito_rpc
        .send_transaction_with_config(swap_tx, send_config)
        .await
        .map_err(|error| error.to_string())
}

fn build_swap_transaction(
    context: &ExecutionContext,
    instructions: Vec<Instruction>,
    blockhash: solana_sdk::hash::Hash,
) -> Result<VersionedTransaction, String> {
    let signer_refs: [&dyn Signer; 1] = [context.keypair.as_ref()];
    TxBuilder::new(context.keypair.pubkey())
        .with_compute_unit_limit(120_000)
        .with_priority_fee_micro_lamports(context.priority_fees)
        .add_instructions(instructions)
        .build_and_sign(blockhash.to_bytes(), &signer_refs)
        .map_err(|error| format!("failed to build/sign swap transaction: {error}"))
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
