use std::{env, net::SocketAddr};
use thiserror::Error;

use crate::domain::{
    config::{ConfigError, SniperConfigFile, load_sniper_config_file},
    value_objects::{
        NonEmptyText, PriorityFeesMicrolamports, ReplayBurstSize, ReplayEventCount,
        SofCommitmentLevel, SofGossipRuntimeMode, SofIngressSource, SofTxJitoTransport, SofTxMode,
        SofTxReliability, SofTxRoute, SofTxStrategy, TxSubmissionMode,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredRuntimeField {
    KeypairPath,
    RpcUrl,
    JitoUrl,
}

impl RequiredRuntimeField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeypairPath => "keypair_path",
            Self::RpcUrl => "rpc_url",
            Self::JitoUrl => "jito_url",
        }
    }
}

impl std::fmt::Display for RequiredRuntimeField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonEmptyRuntimeField {
    WebsocketUrl,
    GrpcUrl,
    PrivateShredSocketPath,
}

impl NonEmptyRuntimeField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::WebsocketUrl => "sof.websocket_url",
            Self::GrpcUrl => "sof.grpc_url",
            Self::PrivateShredSocketPath => "sof.private_shred_socket_path",
        }
    }
}

impl std::fmt::Display for NonEmptyRuntimeField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayField {
    ReplayEventCount,
    ReplayBurstSize,
}

impl ReplayField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReplayEventCount => "replay_event_count",
            Self::ReplayBurstSize => "replay_burst_size",
        }
    }
}

impl std::fmt::Display for ReplayField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryField {
    SampleCapacity,
    ReportPeriodSecs,
}

impl TelemetryField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SampleCapacity => "telemetry.sample_capacity",
            Self::ReportPeriodSecs => "telemetry.report_period_secs",
        }
    }
}

impl std::fmt::Display for TelemetryField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Replay(#[from] ReplaySettingsError),
    #[error(transparent)]
    Runtime(#[from] RuntimeSettingsError),
    #[error(transparent)]
    Telemetry(#[from] TelemetrySettingsError),
}

#[derive(Debug, Error)]
pub enum ReplaySettingsError {
    #[error("{field} must be greater than 0")]
    MustBeGreaterThanZero { field: ReplayField },
}

#[derive(Debug, Error)]
pub enum RuntimeSettingsError {
    #[error("invalid tx_submission_mode; supported values: jito, direct")]
    InvalidTxSubmissionMode,
    #[error("invalid sof.source; supported values: websocket, grpc, private_shred")]
    InvalidSofIngressSource,
    #[error("invalid sof.commitment; supported values: processed, confirmed, finalized")]
    InvalidSofCommitment,
    #[error(
        "invalid sof.gossip_runtime_mode; supported values: full, bootstrap_only, control_plane_only"
    )]
    InvalidSofGossipRuntimeMode,
    #[error("invalid sof.private_shred_source_addr '{value}'")]
    InvalidSofPrivateShredSourceAddr { value: String },
    #[error("invalid sof_tx.mode; supported values: rpc, jito, direct, hybrid, custom")]
    InvalidSofTxMode,
    #[error("invalid sof_tx.strategy; supported values: ordered_fallback, all_at_once")]
    InvalidSofTxStrategy,
    #[error(
        "invalid sof_tx.reliability; supported values: low_latency, balanced, high_reliability"
    )]
    InvalidSofTxReliability,
    #[error("invalid sof_tx.jito_transport; supported values: json_rpc, grpc")]
    InvalidSofTxJitoTransport,
    #[error("invalid sof_tx.routes entry '{value}'; supported values: rpc, jito, direct")]
    InvalidSofTxRoute { value: String },
    #[error("sof_tx.mode=custom requires at least one sof_tx.routes entry")]
    MissingCustomSofTxRoutes,
    #[error("sof_tx routes require at least one route")]
    EmptySofTxRoutes,
    #[error("sof_tx.routing_max_parallel_sends must be greater than 0")]
    InvalidSofTxRoutingMaxParallelSends,
    #[error(
        "sof_tx direct routing requires at least one sof.gossip_entrypoints value; direct uses gossip topology plus runtime.rpc_url leader schedule"
    )]
    MissingSofDirectGossipEntrypoints,
    #[error("sof.ingest_queue_capacity must be greater than 0 when configured")]
    InvalidSofIngestQueueCapacity,
    #[error("legacy ingress has been removed; Slotstrike now requires sof.enabled=true")]
    LegacyIngressRemoved,
    #[error("missing {field} in runtime config")]
    MissingRuntimeField { field: RequiredRuntimeField },
    #[error("{field} must not be empty")]
    EmptyRuntimeField { field: NonEmptyRuntimeField },
}

#[derive(Debug, Error)]
pub enum TelemetrySettingsError {
    #[error("{field} must be greater than 0 when telemetry.enabled=true")]
    InvalidEnabledValue { field: TelemetryField },
}

#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    pub config_path: String,
    pub priority_fees: PriorityFeesMicrolamports,
    pub keypair_path: String,
    pub dry_run: bool,
    pub tx_submission_mode: TxSubmissionMode,
    pub jito_url: String,
    pub rpc_url: String,
    pub sof: SofRuntimeSettings,
    pub sof_tx: SofTxRuntimeSettings,
    pub run_replay_benchmark: bool,
    pub replay_event_count: ReplayEventCount,
    pub replay_burst_size: ReplayBurstSize,
    pub latency_sample_capacity: usize,
    pub latency_slo_ns: u64,
    pub latency_report_period_secs: u64,
    pub telemetry_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct SofRuntimeSettings {
    pub enabled: bool,
    pub source: SofIngressSource,
    pub commitment: SofCommitmentLevel,
    pub websocket_url: Option<NonEmptyText>,
    pub grpc_url: Option<NonEmptyText>,
    pub grpc_x_token: Option<NonEmptyText>,
    pub private_shred_socket_path: Option<NonEmptyText>,
    pub private_shred_source_addr: SocketAddr,
    pub trusted_private_shreds: bool,
    pub gossip_entrypoints: Vec<String>,
    pub gossip_validators: Vec<String>,
    pub gossip_runtime_mode: SofGossipRuntimeMode,
    pub inline_transaction_dispatch: bool,
    pub startup_step_logs: bool,
    pub worker_threads: Option<usize>,
    pub dataset_workers: Option<usize>,
    pub packet_workers: Option<usize>,
    pub ingest_queue_mode: Option<String>,
    pub ingest_queue_capacity: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct SofTxRuntimeSettings {
    pub enabled: bool,
    pub mode: SofTxMode,
    pub strategy: SofTxStrategy,
    pub routes: Vec<SofTxRoute>,
    pub reliability: SofTxReliability,
    pub jito_transport: SofTxJitoTransport,
    pub jito_endpoint: Option<String>,
    pub bundle_only: bool,
    pub routing_next_leaders: usize,
    pub routing_backup_validators: usize,
    pub routing_max_parallel_sends: usize,
    pub guard_require_stable_control_plane: bool,
    pub guard_reject_on_replay_recovery_pending: bool,
    pub guard_max_state_version_drift: u64,
    pub guard_max_opportunity_age_ms: u64,
    pub guard_suppression_ttl_ms: u64,
}

impl RuntimeSettings {
    pub fn from_args() -> Result<Self, SettingsError> {
        let args = env::args().skip(1).collect::<Vec<_>>();
        Self::from_cli_args(&args)
    }

    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self, SettingsError> {
        let config_path =
            arg_value(args, "--config").unwrap_or_else(|| "slotstrike.toml".to_owned());
        let parsed_config = load_sniper_config_file(&config_path)?;
        Self::from_parsed_config(args, config_path, &parsed_config)
    }

    fn from_parsed_config(
        args: &[String],
        config_path: String,
        parsed_config: &SniperConfigFile,
    ) -> Result<Self, SettingsError> {
        let runtime = &parsed_config.runtime;
        let sof = &parsed_config.sof;
        let sof_tx = &parsed_config.sof_tx;
        let telemetry = &parsed_config.telemetry;

        let run_replay_benchmark = arg_flag(args, "--replay-benchmark") || runtime.replay_benchmark;
        let replay_event_count =
            ReplayEventCount::new(runtime.replay_event_count).map_err(|_source| {
                ReplaySettingsError::MustBeGreaterThanZero {
                    field: ReplayField::ReplayEventCount,
                }
            })?;
        let replay_burst_size =
            ReplayBurstSize::new(runtime.replay_burst_size).map_err(|_source| {
                ReplaySettingsError::MustBeGreaterThanZero {
                    field: ReplayField::ReplayBurstSize,
                }
            })?;

        let tx_submission_mode = TxSubmissionMode::parse(&runtime.tx_submission_mode)
            .ok_or(RuntimeSettingsError::InvalidTxSubmissionMode)?;

        if !run_replay_benchmark {
            if runtime.keypair_path.trim().is_empty() {
                return Err(RuntimeSettingsError::MissingRuntimeField {
                    field: RequiredRuntimeField::KeypairPath,
                }
                .into());
            }
            if runtime.rpc_url.trim().is_empty() {
                return Err(RuntimeSettingsError::MissingRuntimeField {
                    field: RequiredRuntimeField::RpcUrl,
                }
                .into());
            }
        }

        let keypair_path = runtime.keypair_path.clone();
        let rpc_url = runtime.rpc_url.clone();
        let jito_url = if run_replay_benchmark {
            runtime.jito_url.clone().unwrap_or_default()
        } else if tx_submission_mode == TxSubmissionMode::Jito {
            runtime
                .jito_url
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or(RuntimeSettingsError::MissingRuntimeField {
                    field: RequiredRuntimeField::JitoUrl,
                })?
        } else {
            runtime.jito_url.clone().unwrap_or_else(|| rpc_url.clone())
        };

        let sof_source = SofIngressSource::parse(&sof.source)
            .ok_or(RuntimeSettingsError::InvalidSofIngressSource)?;
        let sof_commitment = SofCommitmentLevel::parse(&sof.commitment)
            .ok_or(RuntimeSettingsError::InvalidSofCommitment)?;
        let sof_gossip_runtime_mode = SofGossipRuntimeMode::parse(&sof.gossip_runtime_mode)
            .ok_or(RuntimeSettingsError::InvalidSofGossipRuntimeMode)?;
        if !sof.enabled {
            return Err(RuntimeSettingsError::LegacyIngressRemoved.into());
        }
        let sof_websocket_url = optional_non_empty_text(
            sof.websocket_url
                .clone()
                .or_else(|| (!runtime.wss_url.trim().is_empty()).then(|| runtime.wss_url.clone())),
            NonEmptyRuntimeField::WebsocketUrl,
        )?;
        let sof_grpc_url =
            optional_non_empty_text(sof.grpc_url.clone(), NonEmptyRuntimeField::GrpcUrl)?;
        let sof_grpc_x_token =
            optional_non_empty_text(sof.grpc_x_token.clone(), NonEmptyRuntimeField::GrpcUrl)?;
        let sof_private_shred_socket_path = optional_non_empty_text(
            sof.private_shred_socket_path.clone(),
            NonEmptyRuntimeField::PrivateShredSocketPath,
        )?;
        let private_shred_source_addr = sof
            .private_shred_source_addr
            .parse::<SocketAddr>()
            .map_err(
                |_source| RuntimeSettingsError::InvalidSofPrivateShredSourceAddr {
                    value: sof.private_shred_source_addr.clone(),
                },
            )?;

        if let Some(queue_capacity) = sof.ingest_queue_capacity
            && queue_capacity == 0
        {
            return Err(RuntimeSettingsError::InvalidSofIngestQueueCapacity.into());
        }

        let sof_settings = SofRuntimeSettings {
            enabled: sof.enabled,
            source: sof_source,
            commitment: sof_commitment,
            websocket_url: sof_websocket_url,
            grpc_url: sof_grpc_url,
            grpc_x_token: sof_grpc_x_token,
            private_shred_socket_path: sof_private_shred_socket_path,
            private_shred_source_addr,
            trusted_private_shreds: sof.trusted_private_shreds,
            gossip_entrypoints: sof.gossip_entrypoints.clone(),
            gossip_validators: sof.gossip_validators.clone(),
            gossip_runtime_mode: sof_gossip_runtime_mode,
            inline_transaction_dispatch: sof.inline_transaction_dispatch,
            startup_step_logs: sof.startup_step_logs,
            worker_threads: sof.worker_threads,
            dataset_workers: sof.dataset_workers,
            packet_workers: sof.packet_workers,
            ingest_queue_mode: sof.ingest_queue_mode.clone(),
            ingest_queue_capacity: sof.ingest_queue_capacity,
        };

        let sof_tx_mode_raw = sof_tx.mode.as_str();
        let sof_tx_mode =
            SofTxMode::parse(sof_tx_mode_raw).ok_or(RuntimeSettingsError::InvalidSofTxMode)?;
        let sof_tx_strategy = SofTxStrategy::parse(&sof_tx.strategy)
            .ok_or(RuntimeSettingsError::InvalidSofTxStrategy)?;
        let sof_tx_reliability = SofTxReliability::parse(&sof_tx.reliability)
            .ok_or(RuntimeSettingsError::InvalidSofTxReliability)?;
        let sof_tx_jito_transport = SofTxJitoTransport::parse(&sof_tx.jito_transport)
            .ok_or(RuntimeSettingsError::InvalidSofTxJitoTransport)?;
        let sof_tx_routes = resolve_sof_tx_routes(sof_tx_mode, &sof_tx.routes)?;
        if sof_tx.routing_max_parallel_sends == 0 {
            return Err(RuntimeSettingsError::InvalidSofTxRoutingMaxParallelSends.into());
        }
        if sof_tx.enabled
            && sof_tx_routes.contains(&SofTxRoute::Direct)
            && sof.gossip_entrypoints.is_empty()
        {
            return Err(RuntimeSettingsError::MissingSofDirectGossipEntrypoints.into());
        }

        if !run_replay_benchmark {
            match sof_source {
                SofIngressSource::Websocket
                    if sof_settings.enabled && sof_settings.websocket_url.is_none() =>
                {
                    return Err(RuntimeSettingsError::EmptyRuntimeField {
                        field: NonEmptyRuntimeField::WebsocketUrl,
                    }
                    .into());
                }
                SofIngressSource::Grpc
                    if sof_settings.enabled && sof_settings.grpc_url.is_none() =>
                {
                    return Err(RuntimeSettingsError::EmptyRuntimeField {
                        field: NonEmptyRuntimeField::GrpcUrl,
                    }
                    .into());
                }
                SofIngressSource::PrivateShred
                    if sof_settings.enabled && sof_settings.private_shred_socket_path.is_none() =>
                {
                    return Err(RuntimeSettingsError::EmptyRuntimeField {
                        field: NonEmptyRuntimeField::PrivateShredSocketPath,
                    }
                    .into());
                }
                SofIngressSource::Websocket
                | SofIngressSource::Grpc
                | SofIngressSource::PrivateShred => {}
            }
        }

        let sof_tx_settings = SofTxRuntimeSettings {
            enabled: sof_tx.enabled,
            mode: sof_tx_mode,
            strategy: sof_tx_strategy,
            routes: sof_tx_routes,
            reliability: sof_tx_reliability,
            jito_transport: sof_tx_jito_transport,
            jito_endpoint: sof_tx.jito_endpoint.clone(),
            bundle_only: sof_tx.bundle_only,
            routing_next_leaders: sof_tx.routing_next_leaders,
            routing_backup_validators: sof_tx.routing_backup_validators,
            routing_max_parallel_sends: sof_tx.routing_max_parallel_sends,
            guard_require_stable_control_plane: sof_tx.guard_require_stable_control_plane,
            guard_reject_on_replay_recovery_pending: sof_tx.guard_reject_on_replay_recovery_pending,
            guard_max_state_version_drift: sof_tx.guard_max_state_version_drift,
            guard_max_opportunity_age_ms: sof_tx.guard_max_opportunity_age_ms,
            guard_suppression_ttl_ms: sof_tx.guard_suppression_ttl_ms,
        };

        if telemetry.enabled && telemetry.sample_capacity == 0 {
            return Err(TelemetrySettingsError::InvalidEnabledValue {
                field: TelemetryField::SampleCapacity,
            }
            .into());
        }
        if telemetry.enabled && telemetry.report_period_secs == 0 {
            return Err(TelemetrySettingsError::InvalidEnabledValue {
                field: TelemetryField::ReportPeriodSecs,
            }
            .into());
        }

        Ok(Self {
            config_path,
            priority_fees: PriorityFeesMicrolamports::new(runtime.priority_fees),
            keypair_path,
            dry_run: runtime.dry_run,
            tx_submission_mode,
            jito_url,
            rpc_url,
            sof: sof_settings,
            sof_tx: sof_tx_settings,
            run_replay_benchmark,
            replay_event_count,
            replay_burst_size,
            latency_sample_capacity: telemetry.sample_capacity,
            latency_slo_ns: telemetry.slo_ns,
            latency_report_period_secs: telemetry.report_period_secs,
            telemetry_enabled: telemetry.enabled,
        })
    }
}

fn optional_non_empty_text(
    value: Option<String>,
    field: NonEmptyRuntimeField,
) -> Result<Option<NonEmptyText>, RuntimeSettingsError> {
    value
        .map(NonEmptyText::try_from)
        .transpose()
        .map_err(|_source| RuntimeSettingsError::EmptyRuntimeField { field })
}

fn resolve_sof_tx_routes(
    mode: SofTxMode,
    configured_routes: &[String],
) -> Result<Vec<SofTxRoute>, RuntimeSettingsError> {
    if mode == SofTxMode::Custom {
        if configured_routes.is_empty() {
            return Err(RuntimeSettingsError::MissingCustomSofTxRoutes);
        }
        let parsed_routes = configured_routes
            .iter()
            .map(|route| {
                SofTxRoute::parse(route).ok_or_else(|| RuntimeSettingsError::InvalidSofTxRoute {
                    value: route.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if parsed_routes.is_empty() {
            return Err(RuntimeSettingsError::EmptySofTxRoutes);
        }
        return Ok(parsed_routes);
    }

    let routes = match mode {
        SofTxMode::Rpc => vec![SofTxRoute::Rpc],
        SofTxMode::Jito => vec![SofTxRoute::Jito],
        SofTxMode::Direct => vec![SofTxRoute::Direct],
        SofTxMode::Hybrid => vec![SofTxRoute::Direct, SofTxRoute::Rpc],
        SofTxMode::Custom => Vec::new(),
    };

    if routes.is_empty() {
        return Err(RuntimeSettingsError::EmptySofTxRoutes);
    }

    Ok(routes)
}

fn arg_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index.saturating_add(1)))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::RuntimeSettings;
    use crate::domain::{
        config::{ConfigError, SniperConfigFile, parse_sniper_config_toml},
        value_objects::TxSubmissionMode,
    };

    fn minimal_config() -> Result<SniperConfigFile, ConfigError> {
        parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "jito"
jito_url = "https://jito.example"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[telemetry]
enabled = true
sample_capacity = 4096
slo_ns = 1000000
report_period_secs = 15
"#,
        )
    }

    #[test]
    fn parses_sof_only_settings() {
        let config = minimal_config();
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(settings.tx_submission_mode, TxSubmissionMode::Jito);
                assert_eq!(settings.sof.source.as_str(), "websocket");
                assert_eq!(
                    settings.sof.gossip_runtime_mode.as_str(),
                    "control_plane_only"
                );
            }
        }
    }

    #[test]
    fn direct_mode_does_not_require_jito_url() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(settings.tx_submission_mode, TxSubmissionMode::Direct);
                assert_eq!(settings.jito_url, "https://rpc.example");
            }
        }
    }

    #[test]
    fn rejects_unknown_tx_submission_mode() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "unknown"
jito_url = "https://jito.example"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_err());
        }
    }

    #[test]
    fn disabled_telemetry_accepts_zero_capacity_and_report_period() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[telemetry]
enabled = false
sample_capacity = 0
slo_ns = 1000000
report_period_secs = 0
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert!(!settings.telemetry_enabled);
            }
        }
    }

    #[test]
    fn preserves_dry_run_flag() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = true
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert!(settings.dry_run);
            }
        }
    }

    #[test]
    fn private_shred_direct_requires_gossip_entrypoints() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[sof]
enabled = true
source = "private_shred"
private_shred_socket_path = "/tmp/slotstrike-sof-private-shreds.sock"

[sof_tx]
enabled = true
mode = "direct"
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_err());
        }
    }

    #[test]
    fn websocket_direct_is_allowed() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[sof]
enabled = true
source = "websocket"
websocket_url = "wss://wss.example"
gossip_entrypoints = ["127.0.0.1:8001"]

[sof_tx]
enabled = true
mode = "direct"
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
        }
    }

    #[test]
    fn grpc_direct_is_allowed() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = false
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[sof]
enabled = true
source = "grpc"
grpc_url = "http://127.0.0.1:10000"
gossip_entrypoints = ["127.0.0.1:8001"]

[sof_tx]
enabled = true
mode = "direct"
"#,
        );
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
        }
    }
}
