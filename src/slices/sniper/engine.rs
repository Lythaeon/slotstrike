use std::{sync::Arc, time::Instant};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};

use crate::{
    app::context::ExecutionContext,
    domain::{
        aggregates::RuleBook,
        events::{RaydiumCandidateKind, SniperInputEvent, unix_timestamp_now_ns},
    },
};

use super::{cpmm, openbook, telemetry::LatencyTelemetry};

pub struct SniperEngine {
    context: Arc<ExecutionContext>,
    events_rx: mpsc::Receiver<SniperInputEvent>,
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
        events_rx: mpsc::Receiver<SniperInputEvent>,
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
        let mut in_flight = JoinSet::new();
        let worker_limit = event_worker_limit();

        while let Some(event) = self.events_rx.recv().await {
            while in_flight.len() >= worker_limit {
                let _ = in_flight.join_next().await;
            }

            let ingress_to_engine_ns =
                unix_timestamp_now_ns().saturating_sub(event.ingress().normalized_timestamp_ns);
            let context = Arc::clone(&self.context);
            let rulebook = self.rulebook_rx.borrow().clone();
            let telemetry = Arc::clone(&self.telemetry);
            self.telemetry
                .record("ingress_to_engine_ns", ingress_to_engine_ns);

            in_flight.spawn(async move {
                handle_event(context, rulebook, event, telemetry).await;
            });
        }

        while in_flight.join_next().await.is_some() {}

        log::warn!("Log event channel closed. Sniper engine stopped.");
    }
}

async fn handle_event(
    context: Arc<ExecutionContext>,
    rulebook: Arc<RuleBook>,
    event: SniperInputEvent,
    telemetry: Arc<LatencyTelemetry>,
) {
    let classify_started_at = Instant::now();

    match event {
        SniperInputEvent::RaydiumCandidate(event) => {
            let dispatch_started_at = Instant::now();
            match event.kind {
                RaydiumCandidateKind::Cpmm => {
                    cpmm::handle_cpmm_candidate_structured(
                        context,
                        rulebook,
                        event.transaction,
                        event.ingress,
                    )
                    .await;
                }
                RaydiumCandidateKind::OpenBook => {
                    openbook::handle_openbook_candidate_structured(
                        context,
                        rulebook,
                        event.transaction,
                        event.ingress,
                    )
                    .await;
                }
            }
            telemetry.record(
                "strategy_dispatch_ns",
                elapsed_ns_u64(dispatch_started_at.elapsed()),
            );
        }
    }

    telemetry.record(
        "engine_classification_ns",
        elapsed_ns_u64(classify_started_at.elapsed()),
    );
}

trait SniperEventExt {
    fn ingress(&self) -> crate::domain::events::IngressMetadata;
}

impl SniperEventExt for SniperInputEvent {
    fn ingress(&self) -> crate::domain::events::IngressMetadata {
        match self {
            Self::RaydiumCandidate(event) => event.ingress,
        }
    }
}

fn elapsed_ns_u64(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn event_worker_limit() -> usize {
    const MIN_EVENT_WORKERS: usize = 32;
    const MAX_EVENT_WORKERS: usize = 256;
    const MULTIPLIER: usize = 4;

    std::thread::available_parallelism()
        .map(|value| value.get().saturating_mul(MULTIPLIER))
        .unwrap_or(MIN_EVENT_WORKERS)
        .clamp(MIN_EVENT_WORKERS, MAX_EVENT_WORKERS)
}
