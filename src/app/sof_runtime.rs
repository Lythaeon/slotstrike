use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use sof::{
    framework::{
        ObserverPlugin, PluginConfig, PluginHost, TransactionDispatchMode, TransactionEvent,
        TransactionInterest, TransactionPrefilter,
    },
    provider_stream::{
        create_provider_stream_queue,
        websocket::{
            WebsocketTransactionCommitment, WebsocketTransactionConfig, WebsocketTransactionError,
            spawn_websocket_source,
        },
        yellowstone::{
            YellowstoneGrpcCommitment, YellowstoneGrpcConfig, YellowstoneGrpcError,
            spawn_yellowstone_grpc_source,
        },
    },
    runtime::{GossipRuntimeMode, ObserverRuntime, RuntimeSetup, ShredTrustMode},
};
use sof_tx::{
    JitoBlockEngineEndpoint, JitoGrpcTransport, JitoJsonRpcTransport, JitoSubmitConfig,
    RoutingPolicy, SubmitPlan, SubmitReliability, SubmitRoute, SubmitStrategy, TxSubmitClient,
    TxSubmitGuardPolicy, adapters::PluginHostTxProviderAdapter,
};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};
use tokio::{
    net::UnixDatagram,
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use url::Url;

use crate::{
    adapters::raydium::{
        RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID, RaydiumStructuredCandidateKind,
        classify_raydium_creation_instructions,
    },
    app::{
        direct_leader_schedule::{
            DirectLeaderScheduleSafetySource, direct_leader_window_slots,
            spawn_direct_leader_schedule_task,
        },
        errors::IngressStartupError,
    },
    domain::{
        events::{
            IngressMetadata, IngressSource, RaydiumCandidateEvent, RaydiumCandidateKind,
            SniperInputEvent, unix_timestamp_now_ns,
        },
        settings::{RuntimeSettings, SofRuntimeSettings, SofTxRuntimeSettings},
        value_objects::{
            SofCommitmentLevel, SofGossipRuntimeMode, SofIngressSource, SofTxJitoTransport,
            SofTxReliability, SofTxRoute, SofTxStrategy,
        },
    },
};

const PRIVATE_SHRED_BATCH_CAPACITY: usize = 128;

pub struct SofRuntimeHarness {
    pub sof_tx_client: Option<Arc<Mutex<TxSubmitClient>>>,
    pub sof_tx_plan: Option<SubmitPlan>,
    pub sof_tx_uses_jito: bool,
    pub control_plane_adapter: Option<Arc<PluginHostTxProviderAdapter>>,
    runtime: ObserverRuntime,
    background_source: SofBackgroundSource,
    direct_leader_schedule_task: Option<JoinHandle<()>>,
}

type SofTxRuntimeHandle = (
    Option<Arc<Mutex<TxSubmitClient>>>,
    Option<SubmitPlan>,
    bool,
    Option<JoinHandle<()>>,
);

enum SofBackgroundSource {
    Websocket(JoinHandle<Result<(), WebsocketTransactionError>>),
    Grpc(JoinHandle<Result<(), YellowstoneGrpcError>>),
    PrivateShred {
        task: JoinHandle<()>,
        socket_path: PathBuf,
    },
}

impl SofRuntimeHarness {
    pub async fn build(
        settings: &RuntimeSettings,
        events_tx: mpsc::Sender<SniperInputEvent>,
    ) -> Result<Self, IngressStartupError> {
        let cpmm_program =
            parse_pubkey(RAYDIUM_STANDARD_AMM_PROGRAM_ID, "raydium cpmm program id")?;
        let openbook_program = parse_pubkey(RAYDIUM_V4_PROGRAM_ID, "raydium openbook program id")?;
        let candidate_plugin = Arc::new(RaydiumCandidatePlugin::new(
            settings.sof.source,
            settings.sof.commitment,
            settings.sof.inline_transaction_dispatch,
            events_tx,
            cpmm_program,
            openbook_program,
        ));
        let control_plane_adapter = build_control_plane_adapter(settings);
        let mut host_builder = PluginHost::builder().add_shared_plugin(candidate_plugin);
        if let Some(adapter) = &control_plane_adapter {
            host_builder = host_builder.add_shared_plugin(adapter.clone());
        }
        let host = host_builder.build();

        let setup = build_runtime_setup(&settings.sof);
        let (sof_tx_client, sof_tx_plan, sof_tx_uses_jito, direct_leader_schedule_task) =
            build_sof_tx_runtime(settings, control_plane_adapter.clone()).await?;

        match settings.sof.source {
            SofIngressSource::Websocket => {
                let endpoint = settings
                    .sof
                    .websocket_url
                    .as_ref()
                    .map(|value| value.as_str().to_owned())
                    .ok_or_else(|| IngressStartupError::Sof {
                        detail: "missing SOF websocket endpoint".to_owned(),
                    })?;
                let (provider_tx, provider_rx) = create_provider_stream_queue(4_096);
                let config =
                    build_websocket_config(endpoint, settings, cpmm_program, openbook_program);
                let source =
                    spawn_websocket_source(&config, provider_tx)
                        .await
                        .map_err(|error| IngressStartupError::Sof {
                            detail: format!("websocket source bootstrap failed: {error}"),
                        })?;
                let runtime = ObserverRuntime::new()
                    .with_setup(setup)
                    .with_plugin_host(host)
                    .with_provider_stream_ingress(config.runtime_mode(), provider_rx);

                Ok(Self {
                    sof_tx_client,
                    sof_tx_plan,
                    sof_tx_uses_jito,
                    control_plane_adapter,
                    runtime,
                    background_source: SofBackgroundSource::Websocket(source),
                    direct_leader_schedule_task,
                })
            }
            SofIngressSource::Grpc => {
                let endpoint = settings
                    .sof
                    .grpc_url
                    .as_ref()
                    .map(|value| value.as_str().to_owned())
                    .ok_or_else(|| IngressStartupError::Sof {
                        detail: "missing SOF gRPC endpoint".to_owned(),
                    })?;
                let (provider_tx, provider_rx) = create_provider_stream_queue(4_096);
                let config = build_grpc_config(endpoint, settings, cpmm_program, openbook_program);
                let mode = config.runtime_mode();
                let source = spawn_yellowstone_grpc_source(config, provider_tx)
                    .await
                    .map_err(|error| IngressStartupError::Sof {
                        detail: format!("yellowstone gRPC bootstrap failed: {error}"),
                    })?;
                let runtime = ObserverRuntime::new()
                    .with_setup(setup)
                    .with_plugin_host(host)
                    .with_provider_stream_ingress(mode, provider_rx);

                Ok(Self {
                    sof_tx_client,
                    sof_tx_plan,
                    sof_tx_uses_jito,
                    control_plane_adapter,
                    runtime,
                    background_source: SofBackgroundSource::Grpc(source),
                    direct_leader_schedule_task,
                })
            }
            SofIngressSource::PrivateShred => {
                let socket_path = PathBuf::from(
                    settings
                        .sof
                        .private_shred_socket_path
                        .as_ref()
                        .map(|value| value.as_str().to_owned())
                        .ok_or_else(|| IngressStartupError::Sof {
                            detail: "missing SOF private shred socket path".to_owned(),
                        })?,
                );
                let (ingest_tx, ingest_rx) = sof::runtime::create_kernel_bypass_ingress_queue();
                let task = spawn_private_shred_ingest(
                    socket_path.clone(),
                    settings.sof.private_shred_source_addr,
                    ingest_tx,
                )
                .await?;
                let runtime = ObserverRuntime::new()
                    .with_setup(setup)
                    .with_plugin_host(host)
                    .with_kernel_bypass_ingress(ingest_rx);

                Ok(Self {
                    sof_tx_client,
                    sof_tx_plan,
                    sof_tx_uses_jito,
                    control_plane_adapter,
                    runtime,
                    background_source: SofBackgroundSource::PrivateShred { task, socket_path },
                    direct_leader_schedule_task,
                })
            }
        }
    }

    pub async fn run(self) -> Result<(), IngressStartupError> {
        let SofRuntimeHarness {
            runtime,
            background_source,
            direct_leader_schedule_task,
            ..
        } = self;
        let runtime_result = runtime
            .run_until_termination_signal()
            .await
            .map_err(|error| IngressStartupError::Sof {
                detail: format!("runtime exited with error: {error}"),
            });

        if let Some(task) = direct_leader_schedule_task {
            task.abort();
        }
        background_source.shutdown();
        runtime_result
    }
}

impl SofBackgroundSource {
    fn shutdown(self) {
        match self {
            Self::Websocket(handle) => handle.abort(),
            Self::Grpc(handle) => handle.abort(),
            Self::PrivateShred { task, socket_path } => {
                task.abort();
                if let Err(error) = std::fs::remove_file(socket_path) {
                    log::debug!(
                        "failed to remove private shred socket during shutdown: {}",
                        error
                    );
                }
            }
        }
    }
}

struct RaydiumCandidatePlugin {
    ingress_source: SofIngressSource,
    commitment: SofCommitmentLevel,
    inline_dispatch: bool,
    sender: mpsc::Sender<SniperInputEvent>,
    dropped_candidate_events: AtomicU64,
    closed_warned: AtomicBool,
    cpmm_program: Pubkey,
    openbook_program: Pubkey,
    prefilter: TransactionPrefilter,
}

impl RaydiumCandidatePlugin {
    fn new(
        ingress_source: SofIngressSource,
        commitment: SofCommitmentLevel,
        inline_dispatch: bool,
        sender: mpsc::Sender<SniperInputEvent>,
        cpmm_program: Pubkey,
        openbook_program: Pubkey,
    ) -> Self {
        Self {
            ingress_source,
            commitment,
            inline_dispatch,
            sender,
            dropped_candidate_events: AtomicU64::new(0),
            closed_warned: AtomicBool::new(false),
            cpmm_program,
            openbook_program,
            prefilter: TransactionPrefilter::new(TransactionInterest::Critical)
                .with_account_include([cpmm_program, openbook_program]),
        }
    }

    fn enqueue_candidate_event(&self, event: SniperInputEvent) {
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_event)) => {
                let dropped = self
                    .dropped_candidate_events
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                if should_log_drop_count(dropped) {
                    log::warn!(
                        "SOF candidate plugin dropped {} candidate events because the sniper ingress queue is full",
                        dropped
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(_event)) => {
                if !self.closed_warned.swap(true, Ordering::Relaxed) {
                    log::warn!(
                        "SOF candidate plugin could not forward candidate event to sniper engine because the queue is closed"
                    );
                }
            }
        }
    }

    #[cfg(test)]
    fn dropped_candidate_events(&self) -> u64 {
        self.dropped_candidate_events.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ObserverPlugin for RaydiumCandidatePlugin {
    fn name(&self) -> &'static str {
        "slotstrike-raydium-candidate-plugin"
    }

    fn config(&self) -> PluginConfig {
        let config = if self.inline_dispatch {
            PluginConfig::new()
                .with_transaction_mode(TransactionDispatchMode::Inline)
                .with_transaction()
        } else {
            PluginConfig::new().with_transaction()
        };

        config.at_commitment(self.commitment.into())
    }

    fn transaction_prefilter(&self) -> Option<&TransactionPrefilter> {
        Some(&self.prefilter)
    }

    async fn on_transaction(&self, event: &TransactionEvent) {
        if event.signature.is_none() {
            return;
        }

        let kind =
            classify_raydium_candidate(event.tx.as_ref(), self.cpmm_program, self.openbook_program);
        let Some(kind) = kind else {
            return;
        };

        let ingress = IngressMetadata::from_receive_clock(
            self.ingress_source.into(),
            unix_timestamp_now_ns(),
        );
        let event = SniperInputEvent::RaydiumCandidate(RaydiumCandidateEvent {
            kind,
            transaction: Arc::clone(&event.tx),
            ingress,
        });

        self.enqueue_candidate_event(event);
    }
}

const fn should_log_drop_count(dropped: u64) -> bool {
    dropped == 1 || dropped.is_power_of_two()
}

fn classify_raydium_candidate(
    tx: &VersionedTransaction,
    cpmm_program: Pubkey,
    openbook_program: Pubkey,
) -> Option<RaydiumCandidateKind> {
    match classify_raydium_creation_instructions(
        tx.message.static_account_keys(),
        tx.message.instructions(),
        cpmm_program,
        openbook_program,
    ) {
        Some(RaydiumStructuredCandidateKind::Cpmm) => Some(RaydiumCandidateKind::Cpmm),
        Some(RaydiumStructuredCandidateKind::OpenBook) => Some(RaydiumCandidateKind::OpenBook),
        None => None,
    }
}

fn build_runtime_setup(settings: &SofRuntimeSettings) -> RuntimeSetup {
    let mut setup = RuntimeSetup::new()
        .with_inline_transaction_dispatch(settings.inline_transaction_dispatch)
        .with_startup_step_logs(settings.startup_step_logs);

    if let Some(worker_threads) = settings.worker_threads {
        setup = setup.with_worker_threads(worker_threads);
    }
    if let Some(dataset_workers) = settings.dataset_workers {
        setup = setup.with_dataset_workers(dataset_workers);
    }
    if let Some(packet_workers) = settings.packet_workers {
        setup = setup.with_packet_workers(packet_workers);
    }
    if let Some(ingest_queue_mode) = &settings.ingest_queue_mode {
        setup = setup.with_ingest_queue_mode(ingest_queue_mode.clone());
    }
    if let Some(ingest_queue_capacity) = settings.ingest_queue_capacity {
        setup = setup.with_ingest_queue_capacity(ingest_queue_capacity);
    }
    if !settings.gossip_entrypoints.is_empty() {
        setup = setup
            .with_gossip_entrypoints(settings.gossip_entrypoints.clone())
            .with_gossip_runtime_mode(map_gossip_runtime_mode(settings.gossip_runtime_mode));
    }
    if !settings.gossip_validators.is_empty() {
        setup = setup.with_gossip_validators(settings.gossip_validators.clone());
    }
    if settings.source == SofIngressSource::PrivateShred {
        setup = setup.with_shred_trust_mode(if settings.trusted_private_shreds {
            ShredTrustMode::TrustedRawShredProvider
        } else {
            ShredTrustMode::PublicUntrusted
        });
    }

    setup
}

const fn map_gossip_runtime_mode(mode: SofGossipRuntimeMode) -> GossipRuntimeMode {
    match mode {
        SofGossipRuntimeMode::Full => GossipRuntimeMode::Full,
        SofGossipRuntimeMode::BootstrapOnly => GossipRuntimeMode::BootstrapOnly,
        SofGossipRuntimeMode::ControlPlaneOnly => GossipRuntimeMode::ControlPlaneOnly,
    }
}

fn build_websocket_config(
    endpoint: String,
    settings: &RuntimeSettings,
    cpmm_program: Pubkey,
    openbook_program: Pubkey,
) -> WebsocketTransactionConfig {
    WebsocketTransactionConfig::new(endpoint)
        .with_commitment(settings.sof.commitment.into())
        .with_source_instance("slotstrike-websocket")
        .with_vote(false)
        .with_failed(false)
        .with_account_include(vec![cpmm_program, openbook_program])
}

fn build_grpc_config(
    endpoint: String,
    settings: &RuntimeSettings,
    cpmm_program: Pubkey,
    openbook_program: Pubkey,
) -> YellowstoneGrpcConfig {
    let mut config = YellowstoneGrpcConfig::new(endpoint)
        .with_commitment(settings.sof.commitment.into())
        .with_source_instance("slotstrike-yellowstone")
        .with_vote(false)
        .with_failed(false)
        .with_account_include(vec![cpmm_program, openbook_program]);

    if let Some(x_token) = &settings.sof.grpc_x_token {
        config = config.with_x_token(x_token.as_str().to_owned());
    }

    config
}

fn build_control_plane_adapter(
    settings: &RuntimeSettings,
) -> Option<Arc<PluginHostTxProviderAdapter>> {
    if settings.sof.source != SofIngressSource::PrivateShred
        && (!settings.sof_tx.enabled || !settings.sof_tx.routes.contains(&SofTxRoute::Direct))
    {
        return None;
    }

    Some(Arc::new(PluginHostTxProviderAdapter::topology_only(
        sof_tx::adapters::PluginHostTxProviderAdapterConfig {
            max_leader_slots: direct_leader_window_slots(settings.sof_tx.routing_next_leaders),
            max_next_leaders: settings.sof_tx.routing_next_leaders,
        },
    )))
}

async fn build_sof_tx_runtime(
    settings: &RuntimeSettings,
    control_plane_adapter: Option<Arc<PluginHostTxProviderAdapter>>,
) -> Result<SofTxRuntimeHandle, IngressStartupError> {
    if !settings.sof_tx.enabled {
        return Ok((None, None, false, None));
    }

    let plan = submit_plan_from_settings(&settings.sof_tx);
    let uses_jito = plan.routes.contains(&SubmitRoute::Jito);
    let uses_direct = plan.routes.contains(&SubmitRoute::Direct);

    let mut builder = TxSubmitClient::builder()
        .with_rpc_defaults(settings.rpc_url.clone())
        .map_err(|error| IngressStartupError::Sof {
            detail: format!("SOF-TX RPC transport bootstrap failed: {error}"),
        })?
        .with_routing_policy(RoutingPolicy {
            next_leaders: settings.sof_tx.routing_next_leaders,
            backup_validators: settings.sof_tx.routing_backup_validators,
            max_parallel_sends: settings.sof_tx.routing_max_parallel_sends,
        })
        .with_reliability(map_submit_reliability(settings.sof_tx.reliability))
        .with_guard_policy(TxSubmitGuardPolicy {
            require_stable_control_plane: settings.sof_tx.guard_require_stable_control_plane,
            reject_on_replay_recovery_pending: settings
                .sof_tx
                .guard_reject_on_replay_recovery_pending,
            max_state_version_drift: Some(settings.sof_tx.guard_max_state_version_drift),
            max_opportunity_age: Some(std::time::Duration::from_millis(
                settings.sof_tx.guard_max_opportunity_age_ms,
            )),
            suppression_ttl: std::time::Duration::from_millis(
                settings.sof_tx.guard_suppression_ttl_ms,
            ),
        })
        .with_jito_config(JitoSubmitConfig {
            bundle_only: settings.sof_tx.bundle_only,
        });

    if uses_jito {
        builder = builder.with_jito_transport(build_jito_transport(&settings.sof_tx)?);
    }

    if uses_direct {
        let adapter = control_plane_adapter.ok_or_else(|| IngressStartupError::Sof {
            detail: "SOF-TX direct routing requires the SOF plugin-host control-plane adapter"
                .to_owned(),
        })?;
        let direct_leader_schedule_task = Some(spawn_direct_leader_schedule_task(
            settings.rpc_url.clone(),
            settings.sof_tx.routing_next_leaders,
            Arc::clone(&adapter),
        ));
        builder = builder
            .with_blockhash_provider(adapter.clone())
            .with_direct_udp()
            .with_leader_provider(adapter.clone())
            .with_flow_safety_source(Arc::new(DirectLeaderScheduleSafetySource::new(adapter)));

        let client = builder.build();
        return Ok((
            Some(Arc::new(Mutex::new(client))),
            Some(plan),
            uses_jito,
            direct_leader_schedule_task,
        ));
    }

    let client = builder.build();
    Ok((
        Some(Arc::new(Mutex::new(client))),
        Some(plan),
        uses_jito,
        None,
    ))
}

fn build_jito_transport(
    settings: &SofTxRuntimeSettings,
) -> Result<Arc<dyn sof_tx::submit::JitoSubmitTransport>, IngressStartupError> {
    let endpoint = match &settings.jito_endpoint {
        Some(url) => {
            let parsed = Url::parse(url).map_err(|error| IngressStartupError::Sof {
                detail: format!("invalid SOF-TX Jito endpoint '{url}': {error}"),
            })?;
            JitoBlockEngineEndpoint::custom(parsed)
        }
        None => JitoBlockEngineEndpoint::mainnet(),
    };

    match settings.jito_transport {
        SofTxJitoTransport::JsonRpc => JitoJsonRpcTransport::with_endpoint(endpoint)
            .map(|transport| Arc::new(transport) as Arc<dyn sof_tx::submit::JitoSubmitTransport>)
            .map_err(|error| IngressStartupError::Sof {
                detail: format!("failed to configure Jito JSON-RPC transport: {error}"),
            }),
        SofTxJitoTransport::Grpc => JitoGrpcTransport::with_endpoint(endpoint)
            .map(|transport| Arc::new(transport) as Arc<dyn sof_tx::submit::JitoSubmitTransport>)
            .map_err(|error| IngressStartupError::Sof {
                detail: format!("failed to configure Jito gRPC transport: {error}"),
            }),
    }
}

fn submit_plan_from_settings(settings: &SofTxRuntimeSettings) -> SubmitPlan {
    let routes = settings
        .routes
        .iter()
        .copied()
        .map(map_submit_route)
        .collect();
    match settings.strategy {
        SofTxStrategy::OrderedFallback => SubmitPlan::new(routes, SubmitStrategy::OrderedFallback),
        SofTxStrategy::AllAtOnce => SubmitPlan::new(routes, SubmitStrategy::AllAtOnce),
    }
}

const fn map_submit_route(route: SofTxRoute) -> SubmitRoute {
    match route {
        SofTxRoute::Rpc => SubmitRoute::Rpc,
        SofTxRoute::Jito => SubmitRoute::Jito,
        SofTxRoute::Direct => SubmitRoute::Direct,
    }
}

const fn map_submit_reliability(reliability: SofTxReliability) -> SubmitReliability {
    match reliability {
        SofTxReliability::LowLatency => SubmitReliability::LowLatency,
        SofTxReliability::Balanced => SubmitReliability::Balanced,
        SofTxReliability::HighReliability => SubmitReliability::HighReliability,
    }
}

async fn spawn_private_shred_ingest(
    socket_path: PathBuf,
    source_addr: SocketAddr,
    ingest_tx: sof::runtime::KernelBypassIngressSender,
) -> Result<JoinHandle<()>, IngressStartupError> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| IngressStartupError::Sof {
                detail: format!(
                    "failed to create private shred socket directory '{}': {error}",
                    parent.display()
                ),
            })?;
    }

    if Path::new(&socket_path).exists() {
        tokio::fs::remove_file(&socket_path)
            .await
            .map_err(|error| IngressStartupError::Sof {
                detail: format!(
                    "failed to replace private shred socket '{}': {error}",
                    socket_path.display()
                ),
            })?;
    }

    let socket = UnixDatagram::bind(&socket_path).map_err(|error| IngressStartupError::Sof {
        detail: format!(
            "failed to bind private shred socket '{}': {error}",
            socket_path.display()
        ),
    })?;

    Ok(tokio::spawn(async move {
        let mut buffer = [0_u8; sof::ingest::UDP_PACKET_BUFFER_BYTES];

        loop {
            let mut batch =
                sof::ingest::RawPacketBatch::with_capacity(PRIVATE_SHRED_BATCH_CAPACITY);
            match socket.recv(&mut buffer).await {
                Ok(len) => {
                    let Some(payload) = buffer.get(..len) else {
                        continue;
                    };
                    if push_private_shred_packet(&mut batch, source_addr, payload).is_err() {
                        continue;
                    }
                }
                Err(error) => {
                    log::error!("SOF private shred socket receive failed: {}", error);
                    break;
                }
            }

            while batch.len() < PRIVATE_SHRED_BATCH_CAPACITY {
                match socket.try_recv(&mut buffer) {
                    Ok(len) => {
                        let Some(payload) = buffer.get(..len) else {
                            break;
                        };
                        if push_private_shred_packet(&mut batch, source_addr, payload).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        log::error!("SOF private shred socket receive failed: {}", error);
                        break;
                    }
                }
            }

            if !ingest_tx.send_batch(batch, false) {
                log::warn!("SOF private shred ingress queue closed");
                break;
            }
        }
    }))
}

fn push_private_shred_packet(
    batch: &mut sof::ingest::RawPacketBatch,
    source_addr: SocketAddr,
    payload: &[u8],
) -> Result<(), std::io::Error> {
    batch.push_packet_bytes(source_addr, sof::ingest::RawPacketIngress::Udp, payload)
}

fn parse_pubkey(value: &str, label: &str) -> Result<Pubkey, IngressStartupError> {
    Pubkey::from_str(value).map_err(|error| IngressStartupError::Sof {
        detail: format!("invalid {label}: {error}"),
    })
}

impl From<SofCommitmentLevel> for sof::event::TxCommitmentStatus {
    fn from(value: SofCommitmentLevel) -> Self {
        match value {
            SofCommitmentLevel::Processed => Self::Processed,
            SofCommitmentLevel::Confirmed => Self::Confirmed,
            SofCommitmentLevel::Finalized => Self::Finalized,
        }
    }
}

impl From<SofCommitmentLevel> for WebsocketTransactionCommitment {
    fn from(value: SofCommitmentLevel) -> Self {
        match value {
            SofCommitmentLevel::Processed => Self::Processed,
            SofCommitmentLevel::Confirmed => Self::Confirmed,
            SofCommitmentLevel::Finalized => Self::Finalized,
        }
    }
}

impl From<SofCommitmentLevel> for YellowstoneGrpcCommitment {
    fn from(value: SofCommitmentLevel) -> Self {
        match value {
            SofCommitmentLevel::Processed => Self::Processed,
            SofCommitmentLevel::Confirmed => Self::Confirmed,
            SofCommitmentLevel::Finalized => Self::Finalized,
        }
    }
}

impl From<SofIngressSource> for IngressSource {
    fn from(value: SofIngressSource) -> Self {
        match value {
            SofIngressSource::Websocket => Self::Websocket,
            SofIngressSource::Grpc => Self::Grpc,
            SofIngressSource::PrivateShred => Self::PrivateShred,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sof::provider_stream::ProviderStreamMode;
    use solana_sdk::{
        message::Message,
        pubkey::Pubkey,
        transaction::{Transaction, VersionedTransaction},
    };
    use tokio::sync::mpsc;

    use super::{
        RaydiumCandidatePlugin, build_control_plane_adapter, build_grpc_config,
        build_websocket_config,
    };
    use crate::domain::{
        events::{
            IngressMetadata, IngressSource, RaydiumCandidateEvent, RaydiumCandidateKind,
            SniperInputEvent,
        },
        settings::{RuntimeSettings, SofRuntimeSettings, SofTxRuntimeSettings},
        value_objects::{
            PriorityFeesMicrolamports, ReplayBurstSize, ReplayEventCount, SofCommitmentLevel,
            SofGossipRuntimeMode, SofIngressSource, SofTxJitoTransport, SofTxMode,
            SofTxReliability, SofTxRoute, SofTxStrategy, TxSubmissionMode,
        },
    };

    fn runtime_settings() -> Result<RuntimeSettings, &'static str> {
        let private_shred_source_addr = "127.0.0.1:1234"
            .parse()
            .map_err(|_error| "invalid test socket address")?;
        let replay_event_count = ReplayEventCount::new(50_000)?;
        let replay_burst_size = ReplayBurstSize::new(512)?;

        Ok(RuntimeSettings {
            config_path: "slotstrike.toml".to_owned(),
            priority_fees: PriorityFeesMicrolamports::new(1_000),
            keypair_path: "keypair.json".to_owned(),
            dry_run: true,
            tx_submission_mode: TxSubmissionMode::Direct,
            jito_url: "https://jito.example".to_owned(),
            rpc_url: "https://rpc.example".to_owned(),
            sof: SofRuntimeSettings {
                enabled: true,
                source: SofIngressSource::Websocket,
                commitment: SofCommitmentLevel::Processed,
                websocket_url: None,
                grpc_url: None,
                grpc_x_token: None,
                private_shred_socket_path: None,
                private_shred_source_addr,
                trusted_private_shreds: false,
                gossip_entrypoints: vec!["127.0.0.1:8001".to_owned()],
                gossip_validators: Vec::new(),
                gossip_runtime_mode: SofGossipRuntimeMode::ControlPlaneOnly,
                inline_transaction_dispatch: false,
                startup_step_logs: false,
                worker_threads: None,
                dataset_workers: None,
                packet_workers: None,
                ingest_queue_mode: None,
                ingest_queue_capacity: None,
            },
            sof_tx: SofTxRuntimeSettings {
                enabled: true,
                mode: SofTxMode::Direct,
                strategy: SofTxStrategy::OrderedFallback,
                routes: vec![SofTxRoute::Direct],
                reliability: SofTxReliability::Balanced,
                jito_transport: SofTxJitoTransport::JsonRpc,
                jito_endpoint: None,
                bundle_only: true,
                routing_next_leaders: 2,
                routing_backup_validators: 1,
                routing_max_parallel_sends: 4,
                guard_require_stable_control_plane: true,
                guard_reject_on_replay_recovery_pending: true,
                guard_max_state_version_drift: 4,
                guard_max_opportunity_age_ms: 250,
                guard_suppression_ttl_ms: 500,
            },
            run_replay_benchmark: false,
            replay_event_count,
            replay_burst_size,
            latency_sample_capacity: 4_096,
            latency_slo_ns: 1_000_000,
            latency_report_period_secs: 15,
            telemetry_enabled: true,
        })
    }

    #[test]
    fn websocket_direct_keeps_builtin_provider_mode() {
        let settings = runtime_settings();
        assert!(settings.is_ok());
        let settings = match settings {
            Ok(settings) => settings,
            Err(_error) => return,
        };
        let config = build_websocket_config(
            "wss://rpc.example".to_owned(),
            &settings,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        assert_eq!(
            config.runtime_mode(),
            ProviderStreamMode::WebsocketTransaction
        );
    }

    #[test]
    fn grpc_direct_keeps_builtin_provider_mode() {
        let settings = runtime_settings();
        assert!(settings.is_ok());
        let settings = match settings {
            Ok(settings) => settings,
            Err(_error) => return,
        };
        let config = build_grpc_config(
            "http://127.0.0.1:10000".to_owned(),
            &settings,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        assert_eq!(config.runtime_mode(), ProviderStreamMode::YellowstoneGrpc);
    }

    #[test]
    fn control_plane_adapter_is_enabled_for_private_shreds_without_direct_route() {
        let settings = runtime_settings();
        assert!(settings.is_ok());
        let mut settings = match settings {
            Ok(settings) => settings,
            Err(_error) => return,
        };
        settings.sof.source = SofIngressSource::PrivateShred;
        settings.sof_tx.mode = SofTxMode::Rpc;
        settings.sof_tx.routes = vec![SofTxRoute::Rpc];

        assert!(build_control_plane_adapter(&settings).is_some());
    }

    #[test]
    fn control_plane_adapter_is_enabled_for_private_shreds_even_when_sof_tx_is_disabled() {
        let settings = runtime_settings();
        assert!(settings.is_ok());
        let mut settings = match settings {
            Ok(settings) => settings,
            Err(_error) => return,
        };
        settings.sof.source = SofIngressSource::PrivateShred;
        settings.sof_tx.enabled = false;
        settings.sof_tx.mode = SofTxMode::Rpc;
        settings.sof_tx.routes = vec![SofTxRoute::Rpc];

        assert!(build_control_plane_adapter(&settings).is_some());
    }

    #[test]
    fn control_plane_adapter_is_disabled_without_private_shreds_or_direct_route() {
        let settings = runtime_settings();
        assert!(settings.is_ok());
        let mut settings = match settings {
            Ok(settings) => settings,
            Err(_error) => return,
        };
        settings.sof.source = SofIngressSource::Websocket;
        settings.sof_tx.mode = SofTxMode::Jito;
        settings.sof_tx.routes = vec![SofTxRoute::Jito];

        assert!(build_control_plane_adapter(&settings).is_none());
    }

    #[test]
    fn candidate_plugin_drops_when_ingress_queue_is_full() {
        let (sender, mut receiver) = mpsc::channel(1);
        let plugin = candidate_plugin(sender.clone());
        let first_send = sender.try_send(candidate_event(RaydiumCandidateKind::Cpmm));
        assert!(first_send.is_ok());

        plugin.enqueue_candidate_event(candidate_event(RaydiumCandidateKind::OpenBook));

        assert_eq!(plugin.dropped_candidate_events(), 1);
        drop(sender);
        let queued = receiver.try_recv();
        assert!(queued.is_ok());
        assert!(matches!(
            queued,
            Ok(SniperInputEvent::RaydiumCandidate(RaydiumCandidateEvent {
                kind: RaydiumCandidateKind::Cpmm,
                ..
            }))
        ));
    }

    #[test]
    fn candidate_plugin_enqueues_when_queue_has_capacity() {
        let (sender, mut receiver) = mpsc::channel(1);
        let plugin = candidate_plugin(sender);

        plugin.enqueue_candidate_event(candidate_event(RaydiumCandidateKind::OpenBook));

        assert_eq!(plugin.dropped_candidate_events(), 0);
        let queued = receiver.try_recv();
        assert!(matches!(
            queued,
            Ok(SniperInputEvent::RaydiumCandidate(RaydiumCandidateEvent {
                kind: RaydiumCandidateKind::OpenBook,
                ..
            }))
        ));
    }

    fn candidate_plugin(sender: mpsc::Sender<SniperInputEvent>) -> RaydiumCandidatePlugin {
        RaydiumCandidatePlugin::new(
            SofIngressSource::Websocket,
            SofCommitmentLevel::Processed,
            true,
            sender,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        )
    }

    fn candidate_event(kind: RaydiumCandidateKind) -> SniperInputEvent {
        let payer = Pubkey::new_unique();
        let transaction =
            VersionedTransaction::from(Transaction::new_unsigned(Message::new(&[], Some(&payer))));

        SniperInputEvent::RaydiumCandidate(RaydiumCandidateEvent {
            kind,
            transaction: Arc::new(transaction),
            ingress: IngressMetadata::from_receive_clock(IngressSource::Websocket, 1),
        })
    }
}
