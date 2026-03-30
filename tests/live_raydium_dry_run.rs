use std::{error::Error, str::FromStr, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use slotstrike::{
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        entities::SnipeRule,
        events::{IngressMetadata, IngressSource},
        value_objects::{
            RuleAddress, RuleSlippageBps, RuleSolAmount, TxSubmissionMode, sol_amount::Lamports,
        },
    },
    slices::sniper::{cpmm, openbook},
};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcTransactionConfig};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::{Keypair, Signature};
use solana_transaction_status::{
    EncodedTransaction, TransactionBinaryEncoding, UiTransactionEncoding,
};

#[tokio::test]
#[ignore = "requires live RPC access and a historical Raydium pool-creation signature"]
async fn live_raydium_replay_builds_swap_without_submission() -> Result<(), Box<dyn Error>> {
    let rpc_url = std::env::var("SLOTSTRIKE_LIVE_RPC_URL")
        .unwrap_or_else(|_| "https://solana-rpc.publicnode.com".to_owned());
    let signature = std::env::var("SLOTSTRIKE_LIVE_SIGNATURE")?;
    let candidate_kind = std::env::var("SLOTSTRIKE_LIVE_KIND")?;
    let mint = std::env::var("SLOTSTRIKE_LIVE_MINT")?;

    let signature = Signature::from_str(&signature)?;
    let rpc = Arc::new(RpcClient::new(rpc_url.clone()));
    let creation_tx = rpc
        .get_transaction_with_config(
            &signature,
            RpcTransactionConfig {
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: Some(0),
                encoding: Some(UiTransactionEncoding::Base64),
            },
        )
        .await?;
    let transaction = Arc::new(
        decode_transaction(&creation_tx)
            .ok_or_else(|| std::io::Error::other("failed to decode historical transaction"))?,
    );

    let context = Arc::new(ExecutionContext {
        priority_fees: 1,
        rpc,
        keypair: Arc::new(Keypair::new()),
        dry_run: true,
        tx_submission_mode: TxSubmissionMode::Direct,
        jito_url: Arc::new(rpc_url),
        sof_tx_client: None,
        sof_tx_plan: None,
        sof_tx_uses_jito: false,
        sof_tx_blockhash_adapter: None,
        require_local_blockhash: false,
    });
    let rulebook = Arc::new(RuleBook::new(vec![build_mint_rule(&mint)?], Vec::new()));
    let ingress = IngressMetadata::from_receive_clock(
        IngressSource::Grpc,
        slotstrike::domain::events::unix_timestamp_now_ns(),
    );

    let result = tokio::time::timeout(Duration::from_secs(30), async move {
        match candidate_kind.as_str() {
            "cpmm" => {
                cpmm::handle_cpmm_candidate_structured(context, rulebook, transaction, ingress)
                    .await
            }
            "openbook" => {
                openbook::handle_openbook_candidate_structured(
                    context,
                    rulebook,
                    transaction,
                    ingress,
                )
                .await
            }
            value => {
                return Err(std::io::Error::other(format!(
                    "unsupported SLOTSTRIKE_LIVE_KIND '{value}'"
                ))
                .into());
            }
        }

        Ok::<(), Box<dyn Error>>(())
    })
    .await;

    let inner = result
        .map_err(|elapsed| std::io::Error::other(format!("live replay timed out: {elapsed}")))?;
    inner?;
    Ok(())
}

fn build_mint_rule(address: &str) -> Result<SnipeRule, Box<dyn Error>> {
    let address = RuleAddress::try_from(address)?;
    let slippage = RuleSlippageBps::from_pct_str("1")?;
    Ok(SnipeRule::new(
        address,
        RuleSolAmount::new(Lamports::new(100_000_000)),
        RuleSolAmount::new(Lamports::new(1_000_000)),
        slippage,
    ))
}

fn decode_transaction(
    tx: &solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta,
) -> Option<solana_sdk::transaction::VersionedTransaction> {
    let EncodedTransaction::Binary(encoded, TransactionBinaryEncoding::Base64) =
        &tx.transaction.transaction
    else {
        return None;
    };

    let bytes = BASE64_STANDARD.decode(encoded).ok()?;
    bincode::deserialize(&bytes).ok()
}
