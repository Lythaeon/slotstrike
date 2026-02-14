use std::env;

use crate::domain::{
    config::{SniperConfigFile, load_sniper_config_file},
    value_objects::{
        KernelBypassEngine, NonEmptyText, PriorityFeesMicrolamports, ReplayBurstSize,
        ReplayEventCount, TxSubmissionMode,
    },
};

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
    pub fn from_args() -> Result<Self, String> {
        let args = env::args().skip(1).collect::<Vec<_>>();
        Self::from_cli_args(&args)
    }

    pub(crate) fn from_cli_args(args: &[String]) -> Result<Self, String> {
        let config_path = arg_value(args, "--config").unwrap_or_else(|| "slotstrike.toml".to_owned());
        let parsed_config = load_sniper_config_file(&config_path)?;
        Self::from_parsed_config(args, config_path, &parsed_config)
    }

    fn from_parsed_config(
        args: &[String],
        config_path: String,
        parsed_config: &SniperConfigFile,
    ) -> Result<Self, String> {
        let runtime = &parsed_config.runtime;
        let telemetry = &parsed_config.telemetry;

        let run_replay_benchmark = arg_flag(args, "--replay-benchmark") || runtime.replay_benchmark;
        let replay_event_count = ReplayEventCount::new(runtime.replay_event_count)
            .map_err(|message| format!("replay_event_count {}", message))?;
        let replay_burst_size = ReplayBurstSize::new(runtime.replay_burst_size)
            .map_err(|message| format!("replay_burst_size {}", message))?;

        let kernel_tcp_bypass_enabled = runtime.kernel_tcp_bypass;
        let kernel_tcp_bypass_engine = KernelBypassEngine::parse(&runtime.kernel_tcp_bypass_engine)
            .ok_or_else(|| {
                format!(
                    "Invalid kernel_tcp_bypass_engine '{}'. Supported values: af_xdp, dpdk, openonload, af_xdp_or_dpdk_external",
                    runtime.kernel_tcp_bypass_engine
                )
            })?;
        let kernel_bypass_socket_path =
            NonEmptyText::try_from(runtime.kernel_bypass_socket_path.clone())
                .map_err(|_error| "kernel_bypass_socket_path must not be empty".to_owned())?;

        let tx_submission_mode =
            TxSubmissionMode::parse(&runtime.tx_submission_mode).ok_or_else(|| {
                format!(
                    "Invalid tx_submission_mode '{}'",
                    runtime.tx_submission_mode
                )
            })?;

        let fpga_enabled = arg_flag(args, "--fpga") || runtime.fpga_enabled;
        let fpga_verbose = arg_flag(args, "--fpga-verbose") || runtime.fpga_verbose;
        let fpga_vendor = NonEmptyText::try_from(runtime.fpga_vendor.clone())
            .map_err(|_error| "fpga_vendor must not be empty".to_owned())?;

        let network_stack_mode = if fpga_enabled {
            NetworkStackMode::Fpga
        } else if kernel_tcp_bypass_enabled {
            NetworkStackMode::KernelBypass
        } else {
            NetworkStackMode::StandardTcp
        };

        if !run_replay_benchmark {
            if runtime.keypair_path.trim().is_empty() {
                return Err("Missing keypair_path in runtime config".to_owned());
            }
            if runtime.rpc_url.trim().is_empty() {
                return Err("Missing rpc_url in runtime config".to_owned());
            }
            if runtime.wss_url.trim().is_empty() {
                return Err("Missing wss_url in runtime config".to_owned());
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
                .ok_or_else(|| "Missing jito_url for jito submission mode".to_owned())?
        } else {
            runtime.jito_url.clone().unwrap_or_else(|| rpc_url.clone())
        };

        let wss_url_raw = if run_replay_benchmark && runtime.wss_url.trim().is_empty() {
            "wss://localhost".to_owned()
        } else {
            runtime.wss_url.clone()
        };
        let wss_url = NonEmptyText::try_from(wss_url_raw)
            .map_err(|_error| "wss_url must not be empty".to_owned())?;

        if telemetry.enabled && telemetry.sample_capacity == 0 {
            return Err(
                "telemetry.sample_capacity must be greater than 0 when telemetry.enabled=true"
                    .to_owned(),
            );
        }
        if telemetry.enabled && telemetry.report_period_secs == 0 {
            return Err(
                "telemetry.report_period_secs must be greater than 0 when telemetry.enabled=true"
                    .to_owned(),
            );
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
        config::{SniperConfigFile, parse_sniper_config_toml},
        value_objects::{KernelBypassEngine, TxSubmissionMode},
    };

    fn minimal_config() -> Result<SniperConfigFile, String> {
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert_eq!(settings.network_stack_mode, NetworkStackMode::KernelBypass);
                assert_eq!(
                    settings.kernel_tcp_bypass_engine,
                    KernelBypassEngine::AfXdpOrDpdkExternal
                );
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
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
            let settings =
                RuntimeSettings::from_parsed_config(&Vec::new(), "slotstrike.toml".to_owned(), &config);
            assert!(settings.is_ok());
            if let Ok(settings) = settings {
                assert!(!settings.telemetry_enabled);
            }
        }
    }
}
