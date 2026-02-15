use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
pub struct SniperConfigFile {
    pub runtime: RuntimeConfigSection,
    #[serde(default)]
    pub telemetry: TelemetryConfigSection,
    #[serde(default)]
    pub rules: Vec<RuleConfigEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeConfigSection {
    pub keypair_path: String,
    pub rpc_url: String,
    pub wss_url: String,
    pub priority_fees: u64,
    #[serde(default = "default_tx_submission_mode")]
    pub tx_submission_mode: String,
    #[serde(default)]
    pub jito_url: Option<String>,
    #[serde(default = "default_kernel_tcp_bypass")]
    pub kernel_tcp_bypass: bool,
    #[serde(default = "default_kernel_tcp_bypass_engine")]
    pub kernel_tcp_bypass_engine: String,
    #[serde(default = "default_kernel_bypass_socket_path")]
    pub kernel_bypass_socket_path: String,
    #[serde(default)]
    pub fpga_enabled: bool,
    #[serde(default)]
    pub fpga_verbose: bool,
    #[serde(default = "default_fpga_vendor")]
    pub fpga_vendor: String,
    #[serde(default = "default_fpga_ingress_mode")]
    pub fpga_ingress_mode: String,
    #[serde(default = "default_fpga_direct_device_path")]
    pub fpga_direct_device_path: String,
    #[serde(default = "default_fpga_dma_socket_path")]
    pub fpga_dma_socket_path: String,
    #[serde(default)]
    pub replay_benchmark: bool,
    #[serde(default = "default_replay_event_count")]
    pub replay_event_count: usize,
    #[serde(default = "default_replay_burst_size")]
    pub replay_burst_size: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Mint,
    Deployer,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RuleConfigEntry {
    pub kind: RuleKind,
    pub address: String,
    pub snipe_height_sol: String,
    pub tip_budget_sol: String,
    pub slippage_pct: String,
}

#[derive(Clone, Debug, Deserialize)]
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

const fn default_kernel_tcp_bypass() -> bool {
    true
}

fn default_kernel_tcp_bypass_engine() -> String {
    "af_xdp_or_dpdk_external".to_owned()
}

fn default_kernel_bypass_socket_path() -> String {
    "/tmp/slotstrike-kernel-bypass.sock".to_owned()
}

fn default_fpga_vendor() -> String {
    "generic".to_owned()
}

fn default_fpga_ingress_mode() -> String {
    "auto".to_owned()
}

fn default_fpga_direct_device_path() -> String {
    "/dev/slotstrike-fpga0".to_owned()
}

fn default_fpga_dma_socket_path() -> String {
    "/tmp/slotstrike-fpga-dma.sock".to_owned()
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
tx_submission_mode = "direct"
jito_url = "https://jito.example"
kernel_tcp_bypass = false
kernel_tcp_bypass_engine = "af_xdp"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
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
kernel_tcp_bypass = false
kernel_tcp_bypass_engine = "af_xdp"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
replay_benchmark = false
replay_event_count = 50000
replay_burst_size = 512
"#,
        );

        assert!(config.is_ok());
        if let Ok(config) = config {
            assert!(config.telemetry.enabled);
        }
    }
}
