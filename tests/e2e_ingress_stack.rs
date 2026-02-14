use std::{sync::Arc, time::Duration};

use slotstrike::{
    adapters::{fpga_feed::FpgaFeedAdapter, raydium::RAYDIUM_V4_PROGRAM_ID},
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{IngressMetadata, IngressSource, RawLogEvent, unix_timestamp_now_ns},
        value_objects::TxSubmissionMode,
    },
    ports::fpga_feed::FpgaFeedPort,
    slices::sniper::{engine::SniperEngine, telemetry::LatencyTelemetry},
};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::signature::{Keypair, Signature};
use tokio::sync::{mpsc, watch};

#[tokio::test]
async fn e2e_kernel_bypass_software_stack_records_latency() {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (rulebook_tx, rulebook_rx) = watch::channel(Arc::new(RuleBook::default()));
    let telemetry = Arc::new(LatencyTelemetry::new(128, 1_000_000_000));

    let engine = SniperEngine::new(
        test_context(),
        events_rx,
        rulebook_rx,
        Arc::clone(&telemetry),
    );
    let engine_task = tokio::spawn(async move {
        engine.run().await;
    });

    let send_result = events_tx.send(RawLogEvent {
        signature: Signature::new_unique().to_string(),
        logs: vec!["Program log: not_a_pool_creation_event".to_owned()],
        has_error: false,
        ingress: IngressMetadata::from_receive_clock(
            IngressSource::KernelBypass,
            unix_timestamp_now_ns(),
        ),
    });
    assert!(send_result.is_ok());

    drop(events_tx);
    drop(rulebook_tx);

    assert!(wait_for_hop_samples(&telemetry, "engine_classification_ns", 1).await);
    assert!(wait_for_hop_samples(&telemetry, "ingress_to_engine_ns", 1).await);

    let joined = tokio::time::timeout(Duration::from_secs(1), engine_task).await;
    assert!(joined.is_ok());
    if let Ok(joined) = joined {
        assert!(joined.is_ok());
    }
}

#[tokio::test]
async fn e2e_fpga_dma_stack_records_latency_without_network_calls() {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (rulebook_tx, rulebook_rx) = watch::channel(Arc::new(RuleBook::default()));
    let telemetry = Arc::new(LatencyTelemetry::new(128, 1_000_000_000));

    let engine = SniperEngine::new(
        test_context(),
        events_rx,
        rulebook_rx,
        Arc::clone(&telemetry),
    );
    let engine_task = tokio::spawn(async move {
        engine.run().await;
    });

    let payload = format!(
        "signature=not_base58_signature!\nhas_error=0\nlog=Program {}\nlog=Program log: initialize2",
        RAYDIUM_V4_PROGRAM_ID
    );
    let adapter =
        FpgaFeedAdapter::with_mock_dma_payload("mock_dma".to_owned(), true, payload.as_bytes());

    let spawned = adapter.spawn_stream(events_tx.clone());
    assert!(spawned.is_ok());

    drop(events_tx);
    drop(rulebook_tx);

    assert!(wait_for_hop_samples(&telemetry, "engine_classification_ns", 1).await);
    assert!(wait_for_hop_samples(&telemetry, "ingress_to_engine_ns", 1).await);

    let joined = tokio::time::timeout(Duration::from_secs(1), engine_task).await;
    assert!(joined.is_ok());
    if let Ok(joined) = joined {
        assert!(joined.is_ok());
    }
}

async fn wait_for_hop_samples(
    telemetry: &LatencyTelemetry,
    hop: &'static str,
    minimum_samples: usize,
) -> bool {
    for _ in 0..25 {
        let matched = telemetry
            .snapshot_all()
            .into_iter()
            .any(|(name, stats)| name == hop && stats.sample_count >= minimum_samples);
        if matched {
            return true;
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    false
}

fn test_context() -> Arc<ExecutionContext> {
    Arc::new(ExecutionContext {
        priority_fees: 1,
        rpc: Arc::new(RpcClient::new("http://127.0.0.1:8899".to_owned())),
        keypair: Arc::new(Keypair::new()),
        tx_submission_mode: TxSubmissionMode::Direct,
        jito_url: Arc::new("http://127.0.0.1:8899".to_owned()),
    })
}
