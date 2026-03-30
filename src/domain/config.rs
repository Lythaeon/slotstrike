use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
pub struct SniperConfigFile {
    pub runtime: RuntimeConfigSection,
    #[serde(default)]
    pub sof: SofConfigSection,
    #[serde(default)]
    pub sof_tx: SofTxConfigSection,
    #[serde(default)]
    pub telemetry: TelemetryConfigSection,
    #[serde(default)]
    pub rules: Vec<RuleConfigEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfigSection {
    pub keypair_path: String,
    pub rpc_url: String,
    pub wss_url: String,
    pub priority_fees: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_tx_submission_mode")]
    pub tx_submission_mode: String,
    #[serde(default)]
    pub jito_url: Option<String>,
    #[serde(default)]
    pub replay_benchmark: bool,
    #[serde(default = "default_replay_event_count")]
    pub replay_event_count: usize,
    #[serde(default = "default_replay_burst_size")]
    pub replay_burst_size: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SofConfigSection {
    #[serde(default = "default_sof_enabled")]
    pub enabled: bool,
    #[serde(default = "default_sof_source")]
    pub source: String,
    #[serde(default)]
    pub websocket_url: Option<String>,
    #[serde(default)]
    pub grpc_url: Option<String>,
    #[serde(default)]
    pub grpc_x_token: Option<String>,
    #[serde(default)]
    pub private_shred_socket_path: Option<String>,
    #[serde(default = "default_sof_private_shred_source_addr")]
    pub private_shred_source_addr: String,
    #[serde(default)]
    pub trusted_private_shreds: bool,
    #[serde(default)]
    pub gossip_entrypoints: Vec<String>,
    #[serde(default)]
    pub gossip_validators: Vec<String>,
    #[serde(default = "default_sof_gossip_runtime_mode")]
    pub gossip_runtime_mode: String,
    #[serde(default = "default_sof_commitment")]
    pub commitment: String,
    #[serde(default = "default_sof_inline_transaction_dispatch")]
    pub inline_transaction_dispatch: bool,
    #[serde(default)]
    pub startup_step_logs: bool,
    #[serde(default)]
    pub worker_threads: Option<usize>,
    #[serde(default)]
    pub dataset_workers: Option<usize>,
    #[serde(default)]
    pub packet_workers: Option<usize>,
    #[serde(default)]
    pub ingest_queue_mode: Option<String>,
    #[serde(default)]
    pub ingest_queue_capacity: Option<usize>,
}

impl Default for SofConfigSection {
    fn default() -> Self {
        Self {
            enabled: default_sof_enabled(),
            source: default_sof_source(),
            websocket_url: None,
            grpc_url: None,
            grpc_x_token: None,
            private_shred_socket_path: None,
            private_shred_source_addr: default_sof_private_shred_source_addr(),
            trusted_private_shreds: false,
            gossip_entrypoints: Vec::new(),
            gossip_validators: Vec::new(),
            gossip_runtime_mode: default_sof_gossip_runtime_mode(),
            commitment: default_sof_commitment(),
            inline_transaction_dispatch: default_sof_inline_transaction_dispatch(),
            startup_step_logs: false,
            worker_threads: None,
            dataset_workers: None,
            packet_workers: None,
            ingest_queue_mode: None,
            ingest_queue_capacity: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SofTxConfigSection {
    #[serde(default = "default_sof_tx_enabled")]
    pub enabled: bool,
    #[serde(default = "default_sof_tx_mode")]
    pub mode: String,
    #[serde(default = "default_sof_tx_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default = "default_sof_tx_reliability")]
    pub reliability: String,
    #[serde(default = "default_sof_tx_jito_transport")]
    pub jito_transport: String,
    #[serde(default)]
    pub jito_endpoint: Option<String>,
    #[serde(default = "default_sof_tx_bundle_only")]
    pub bundle_only: bool,
    #[serde(default = "default_sof_tx_routing_next_leaders")]
    pub routing_next_leaders: usize,
    #[serde(default = "default_sof_tx_routing_backup_validators")]
    pub routing_backup_validators: usize,
    #[serde(default = "default_sof_tx_routing_max_parallel_sends")]
    pub routing_max_parallel_sends: usize,
    #[serde(default = "default_sof_tx_guard_require_stable_control_plane")]
    pub guard_require_stable_control_plane: bool,
    #[serde(default = "default_sof_tx_guard_reject_on_replay_recovery_pending")]
    pub guard_reject_on_replay_recovery_pending: bool,
    #[serde(default = "default_sof_tx_guard_max_state_version_drift")]
    pub guard_max_state_version_drift: u64,
    #[serde(default = "default_sof_tx_guard_max_opportunity_age_ms")]
    pub guard_max_opportunity_age_ms: u64,
    #[serde(default = "default_sof_tx_guard_suppression_ttl_ms")]
    pub guard_suppression_ttl_ms: u64,
}

impl Default for SofTxConfigSection {
    fn default() -> Self {
        Self {
            enabled: default_sof_tx_enabled(),
            mode: default_sof_tx_mode(),
            strategy: default_sof_tx_strategy(),
            routes: Vec::new(),
            reliability: default_sof_tx_reliability(),
            jito_transport: default_sof_tx_jito_transport(),
            jito_endpoint: None,
            bundle_only: default_sof_tx_bundle_only(),
            routing_next_leaders: default_sof_tx_routing_next_leaders(),
            routing_backup_validators: default_sof_tx_routing_backup_validators(),
            routing_max_parallel_sends: default_sof_tx_routing_max_parallel_sends(),
            guard_require_stable_control_plane: default_sof_tx_guard_require_stable_control_plane(),
            guard_reject_on_replay_recovery_pending:
                default_sof_tx_guard_reject_on_replay_recovery_pending(),
            guard_max_state_version_drift: default_sof_tx_guard_max_state_version_drift(),
            guard_max_opportunity_age_ms: default_sof_tx_guard_max_opportunity_age_ms(),
            guard_suppression_ttl_ms: default_sof_tx_guard_suppression_ttl_ms(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Mint,
    Deployer,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfigEntry {
    pub kind: RuleKind,
    pub address: String,
    pub snipe_height_sol: String,
    pub tip_budget_sol: String,
    pub slippage_pct: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfigSection {
    #[serde(default = "default_telemetry_enabled")]
    pub enabled: bool,
    #[serde(default = "default_telemetry_sample_capacity")]
    pub sample_capacity: usize,
    #[serde(default = "default_telemetry_slo_ns")]
    pub slo_ns: u64,
    #[serde(default = "default_telemetry_report_period_secs")]
    pub report_period_secs: u64,
}

impl Default for TelemetryConfigSection {
    fn default() -> Self {
        Self {
            enabled: default_telemetry_enabled(),
            sample_capacity: default_telemetry_sample_capacity(),
            slo_ns: default_telemetry_slo_ns(),
            report_period_secs: default_telemetry_report_period_secs(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file at {path}")]
    ReadConfigFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid slotstrike.toml format")]
    ParseToml {
        #[source]
        source: toml::de::Error,
    },
}

pub fn load_sniper_config_file(path: &str) -> Result<SniperConfigFile, ConfigError> {
    let config_text =
        std::fs::read_to_string(path).map_err(|source| ConfigError::ReadConfigFile {
            path: PathBuf::from(path),
            source,
        })?;
    parse_sniper_config_toml(&config_text)
}

pub fn parse_sniper_config_toml(config_text: &str) -> Result<SniperConfigFile, ConfigError> {
    toml::from_str::<SniperConfigFile>(config_text)
        .map_err(|source| ConfigError::ParseToml { source })
}

fn default_tx_submission_mode() -> String {
    "jito".to_owned()
}

const fn default_sof_enabled() -> bool {
    true
}

fn default_sof_source() -> String {
    "websocket".to_owned()
}

fn default_sof_private_shred_source_addr() -> String {
    "127.0.0.1:8899".to_owned()
}

fn default_sof_commitment() -> String {
    "processed".to_owned()
}

fn default_sof_gossip_runtime_mode() -> String {
    "control_plane_only".to_owned()
}

const fn default_sof_inline_transaction_dispatch() -> bool {
    true
}

const fn default_sof_tx_enabled() -> bool {
    true
}

fn default_sof_tx_mode() -> String {
    "jito".to_owned()
}

fn default_sof_tx_strategy() -> String {
    "ordered_fallback".to_owned()
}

fn default_sof_tx_reliability() -> String {
    "balanced".to_owned()
}

fn default_sof_tx_jito_transport() -> String {
    "json_rpc".to_owned()
}

const fn default_sof_tx_bundle_only() -> bool {
    true
}

const fn default_sof_tx_routing_next_leaders() -> usize {
    2
}

const fn default_sof_tx_routing_backup_validators() -> usize {
    1
}

const fn default_sof_tx_routing_max_parallel_sends() -> usize {
    4
}

const fn default_sof_tx_guard_require_stable_control_plane() -> bool {
    true
}

const fn default_sof_tx_guard_reject_on_replay_recovery_pending() -> bool {
    true
}

const fn default_sof_tx_guard_max_state_version_drift() -> u64 {
    4
}

const fn default_sof_tx_guard_max_opportunity_age_ms() -> u64 {
    750
}

const fn default_sof_tx_guard_suppression_ttl_ms() -> u64 {
    750
}

const fn default_replay_event_count() -> usize {
    50_000
}

const fn default_replay_burst_size() -> usize {
    512
}

const fn default_telemetry_sample_capacity() -> usize {
    4_096
}

const fn default_telemetry_enabled() -> bool {
    true
}

const fn default_telemetry_slo_ns() -> u64 {
    1_000_000
}

const fn default_telemetry_report_period_secs() -> u64 {
    15
}

#[cfg(test)]
mod tests {
    use super::{RuleKind, parse_sniper_config_toml};

    #[test]
    fn parses_runtime_and_rules_from_toml() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
dry_run = true
tx_submission_mode = "direct"
jito_url = "https://jito.example"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512

[telemetry]
enabled = true
sample_capacity = 1024
slo_ns = 500000
report_period_secs = 5

[[rules]]
kind = "mint"
address = "So11111111111111111111111111111111111111112"
snipe_height_sol = "0.01"
tip_budget_sol = "0.001"
slippage_pct = "1"
"#,
        );

        assert!(config.is_ok());
        if let Ok(config) = config {
            assert_eq!(config.runtime.priority_fees, 1_000);
            assert!(config.runtime.dry_run);
            assert!(config.telemetry.enabled);
            assert_eq!(config.telemetry.sample_capacity, 1_024);
            assert_eq!(config.rules.len(), 1);
            if let Some(rule) = config.rules.first() {
                assert_eq!(rule.kind, RuleKind::Mint);
            }
        }
    }

    #[test]
    fn telemetry_enabled_defaults_to_true() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "direct"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );

        assert!(config.is_ok());
        if let Ok(config) = config {
            assert!(!config.runtime.dry_run);
            assert!(config.telemetry.enabled);
        }
    }

    #[test]
    fn rejects_legacy_runtime_fields() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "direct"
kernel_tcp_bypass = true
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );

        assert!(config.is_err());
    }
}
