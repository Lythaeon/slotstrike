use std::{str::FromStr, sync::Arc, time::Instant};

use solana_sdk::signature::Signature;
use tokio::sync::{mpsc, watch};

use crate::{
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{RawLogEvent, unix_timestamp_now_ns},
    },
};

use super::{
    cpmm, openbook,
    pool_filter::{is_cpmm_candidate_logs, is_openbook_candidate_logs},
    telemetry::LatencyTelemetry,
};

pub struct SniperEngine {
    context: Arc<ExecutionContext>,
    events_rx: mpsc::UnboundedReceiver<RawLogEvent>,
    rulebook_rx: watch::Receiver<Arc<RuleBook>>,
    telemetry: Arc<LatencyTelemetry>,
}

impl SniperEngine {
    #[expect(
        clippy::missing_const_for_fn,
        reason = "runtime initialization with channels and Arcs"
    )]
    pub fn new(
        context: Arc<ExecutionContext>,
        events_rx: mpsc::UnboundedReceiver<RawLogEvent>,
        rulebook_rx: watch::Receiver<Arc<RuleBook>>,
        telemetry: Arc<LatencyTelemetry>,
    ) -> Self {
        Self {
            context,
            events_rx,
            rulebook_rx,
            telemetry,
        }
    }

    pub async fn run(mut self) {
        while let Some(event) = self.events_rx.recv().await {
            let context = Arc::clone(&self.context);
            let rulebook = self.rulebook_rx.borrow().clone();
            let telemetry = Arc::clone(&self.telemetry);
            let ingress_to_engine_ns =
                unix_timestamp_now_ns().saturating_sub(event.ingress.normalized_timestamp_ns);
            self.telemetry
                .record("ingress_to_engine_ns", ingress_to_engine_ns);

            tokio::spawn(async move {
                handle_event(context, rulebook, event, telemetry).await;
            });
        }

        log::warn!("Log event channel closed. Sniper engine stopped.");
    }
}

async fn handle_event(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    event: RawLogEvent,
    telemetry: Arc<LatencyTelemetry>,
) {
    let classify_started_at = Instant::now();

    if event.has_error {
        telemetry.record(
            "engine_classification_ns",
            elapsed_ns_u64(classify_started_at.elapsed()),
        );
        return;
    }

    let signature = match Signature::from_str(&event.signature) {
        Ok(value) => value,
        Err(error) => {
            log::debug!("Invalid signature in log event: {}", error);
            telemetry.record(
                "engine_classification_ns",
                elapsed_ns_u64(classify_started_at.elapsed()),
            );
            return;
        }
    };

    let dispatch_started_at = Instant::now();
    let ingress_metadata = event.ingress;
    let logs = event.logs;

    if is_cpmm_candidate_logs(&logs) {
        cpmm::handle_cpmm_event(context, rulebook, logs, signature, ingress_metadata).await;
        telemetry.record(
            "strategy_dispatch_ns",
            elapsed_ns_u64(dispatch_started_at.elapsed()),
        );
        telemetry.record(
            "engine_classification_ns",
            elapsed_ns_u64(classify_started_at.elapsed()),
        );
        return;
    }

    if is_openbook_candidate_logs(&logs) {
        openbook::handle_openbook_event(context, rulebook, logs, signature, ingress_metadata).await;
        telemetry.record(
            "strategy_dispatch_ns",
            elapsed_ns_u64(dispatch_started_at.elapsed()),
        );
    }

    telemetry.record(
        "engine_classification_ns",
        elapsed_ns_u64(classify_started_at.elapsed()),
    );
}

fn elapsed_ns_u64(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
