use std::env;
use thiserror::Error;

use crate::domain::{
    config::{ConfigError, SniperConfigFile, load_sniper_config_file},
    value_objects::{
        FpgaIngressMode, KernelBypassEngine, NonEmptyText, PriorityFeesMicrolamports,
        ReplayBurstSize, ReplayEventCount, TxSubmissionMode,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredRuntimeField {
    KeypairPath,
    RpcUrl,
    WssUrl,
    JitoUrl,
}

impl RequiredRuntimeField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KeypairPath => "keypair_path",
            Self::RpcUrl => "rpc_url",
            Self::WssUrl => "wss_url",
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
    KernelBypassSocketPath,
    FpgaVendor,
    FpgaDirectDevicePath,
    FpgaDmaSocketPath,
    WssUrl,
}

impl NonEmptyRuntimeField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::KernelBypassSocketPath => "kernel_bypass_socket_path",
            Self::FpgaVendor => "fpga_vendor",
            Self::FpgaDirectDevicePath => "fpga_direct_device_path",
            Self::FpgaDmaSocketPath => "fpga_dma_socket_path",
            Self::WssUrl => "wss_url",
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
    #[error(
        "invalid kernel_tcp_bypass_engine; supported values: af_xdp, dpdk, openonload, af_xdp_or_dpdk_external"
    )]
    InvalidKernelBypassEngine,
    #[error(
        "invalid fpga_ingress_mode; supported values: auto, mock_dma, direct_device, external_socket"
    )]
    InvalidFpgaIngressMode,
    #[error("invalid tx_submission_mode; supported values: jito, direct")]
    InvalidTxSubmissionMode,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkStackMode {
    Fpga,
    KernelBypass,
    StandardTcp,
}

impl NetworkStackMode {
    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fpga => "fpga",
            Self::KernelBypass => "kernel_bypass",
            Self::StandardTcp => "standard_tcp",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    pub config_path: String,
    pub priority_fees: PriorityFeesMicrolamports,
    pub keypair_path: String,
    pub tx_submission_mode: TxSubmissionMode,
    pub jito_url: String,
    pub rpc_url: String,
    pub wss_url: NonEmptyText,
    pub kernel_tcp_bypass_enabled: bool,
    pub kernel_tcp_bypass_engine: KernelBypassEngine,
    pub kernel_bypass_socket_path: NonEmptyText,
    pub fpga_enabled: bool,
    pub fpga_verbose: bool,
    pub fpga_vendor: NonEmptyText,
    pub fpga_ingress_mode: FpgaIngressMode,
    pub fpga_direct_device_path: NonEmptyText,
    pub fpga_dma_socket_path: NonEmptyText,
    pub network_stack_mode: NetworkStackMode,
    pub run_replay_benchmark: bool,
    pub replay_event_count: ReplayEventCount,
    pub replay_burst_size: ReplayBurstSize,
    pub latency_sample_capacity: usize,
    pub latency_slo_ns: u64,
    pub latency_report_period_secs: u64,
    pub telemetry_enabled: bool,
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

        let kernel_tcp_bypass_enabled = runtime.kernel_tcp_bypass;
        let kernel_tcp_bypass_engine = KernelBypassEngine::parse(&runtime.kernel_tcp_bypass_engine)
            .ok_or(RuntimeSettingsError::InvalidKernelBypassEngine)?;
        let kernel_bypass_socket_path = NonEmptyText::try_from(
            runtime.kernel_bypass_socket_path.clone(),
        )
        .map_err(|_source| RuntimeSettingsError::EmptyRuntimeField {
            field: NonEmptyRuntimeField::KernelBypassSocketPath,
        })?;

        let tx_submission_mode = TxSubmissionMode::parse(&runtime.tx_submission_mode)
            .ok_or(RuntimeSettingsError::InvalidTxSubmissionMode)?;

        let fpga_enabled = arg_flag(args, "--fpga") || runtime.fpga_enabled;
        let fpga_verbose = arg_flag(args, "--fpga-verbose") || runtime.fpga_verbose;
        let fpga_vendor =
            NonEmptyText::try_from(runtime.fpga_vendor.clone()).map_err(|_source| {
                RuntimeSettingsError::EmptyRuntimeField {
                    field: NonEmptyRuntimeField::FpgaVendor,
                }
            })?;
        let fpga_ingress_mode = FpgaIngressMode::parse(&runtime.fpga_ingress_mode)
            .ok_or(RuntimeSettingsError::InvalidFpgaIngressMode)?;
        let fpga_direct_device_path =
            NonEmptyText::try_from(runtime.fpga_direct_device_path.clone()).map_err(|_source| {
                RuntimeSettingsError::EmptyRuntimeField {
                    field: NonEmptyRuntimeField::FpgaDirectDevicePath,
                }
            })?;
        let fpga_dma_socket_path = NonEmptyText::try_from(runtime.fpga_dma_socket_path.clone())
            .map_err(|_source| RuntimeSettingsError::EmptyRuntimeField {
                field: NonEmptyRuntimeField::FpgaDmaSocketPath,
            })?;

        let network_stack_mode = if fpga_enabled {
            NetworkStackMode::Fpga
        } else if kernel_tcp_bypass_enabled {
            NetworkStackMode::KernelBypass
        } else {
            NetworkStackMode::StandardTcp
        };

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
            if runtime.wss_url.trim().is_empty() {
                return Err(RuntimeSettingsError::MissingRuntimeField {
                    field: RequiredRuntimeField::WssUrl,
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

        let wss_url_raw = if run_replay_benchmark && runtime.wss_url.trim().is_empty() {
            "wss://localhost".to_owned()
        } else {
            runtime.wss_url.clone()
        };
        let wss_url = NonEmptyText::try_from(wss_url_raw).map_err(|_source| {
            RuntimeSettingsError::EmptyRuntimeField {
                field: NonEmptyRuntimeField::WssUrl,
            }
        })?;

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
            tx_submission_mode,
            jito_url,
            rpc_url,
            wss_url,
            kernel_tcp_bypass_enabled,
            kernel_tcp_bypass_engine,
            kernel_bypass_socket_path,
            fpga_enabled,
            fpga_verbose,
            fpga_vendor,
            fpga_ingress_mode,
            fpga_direct_device_path,
            fpga_dma_socket_path,
            network_stack_mode,
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
    use super::{NetworkStackMode, RuntimeSettings};
    use crate::domain::{
        config::{ConfigError, SniperConfigFile, parse_sniper_config_toml},
        value_objects::{FpgaIngressMode, KernelBypassEngine, TxSubmissionMode},
    };

    fn minimal_config() -> Result<SniperConfigFile, ConfigError> {
        parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "jito"
jito_url = "https://jito.example"
kernel_tcp_bypass = true
kernel_tcp_bypass_engine = "af_xdp_or_dpdk_external"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
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
    fn defaults_to_kernel_bypass_mode() {
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
                assert_eq!(settings.network_stack_mode, NetworkStackMode::KernelBypass);
                assert_eq!(
                    settings.kernel_tcp_bypass_engine,
                    KernelBypassEngine::AfXdpOrDpdkExternal
                );
                assert_eq!(settings.fpga_ingress_mode, FpgaIngressMode::Auto);
                assert_eq!(settings.tx_submission_mode, TxSubmissionMode::Jito);
            }
        }
    }

    #[test]
    fn enables_fpga_mode_from_flag() {
        let config = minimal_config();
        assert!(config.is_ok());
        if let Ok(config) = config {
            let settings = RuntimeSettings::from_parsed_config(
                &["--fpga".to_owned()],
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(settings.network_stack_mode, NetworkStackMode::Fpga);
                assert!(settings.fpga_enabled);
            }
        }
    }

    #[test]
    fn supports_standard_tcp_mode_when_bypass_disabled() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "jito"
jito_url = "https://jito.example"
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
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(settings.network_stack_mode, NetworkStackMode::StandardTcp);
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
tx_submission_mode = "direct"
kernel_tcp_bypass = true
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
    fn accepts_openonload_kernel_engine() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "direct"
kernel_tcp_bypass = true
kernel_tcp_bypass_engine = "openonload"
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
            let settings = RuntimeSettings::from_parsed_config(
                &Vec::new(),
                "slotstrike.toml".to_owned(),
                &config,
            );
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(
                    settings.kernel_tcp_bypass_engine,
                    KernelBypassEngine::OpenOnload
                );
                assert_eq!(settings.network_stack_mode, NetworkStackMode::KernelBypass);
            }
        }
    }

    #[test]
    fn rejects_unknown_fpga_ingress_mode() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "direct"
kernel_tcp_bypass = true
kernel_tcp_bypass_engine = "af_xdp"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
fpga_ingress_mode = "invalid"
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
    fn rejects_unknown_tx_submission_mode() {
        let config = parse_sniper_config_toml(
            r#"
[runtime]
keypair_path = "keypair.json"
rpc_url = "https://rpc.example"
wss_url = "wss://wss.example"
priority_fees = 1000
tx_submission_mode = "unknown"
jito_url = "https://jito.example"
kernel_tcp_bypass = true
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
tx_submission_mode = "direct"
kernel_tcp_bypass = true
kernel_tcp_bypass_engine = "af_xdp"
fpga_enabled = false
fpga_verbose = false
fpga_vendor = "generic"
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
}
