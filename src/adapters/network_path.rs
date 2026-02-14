use crate::{
    domain::{
        settings::{NetworkStackMode, RuntimeSettings},
        value_objects::{KernelBypassEngine, NonEmptyText},
    },
    ports::network_path::NetworkPathPort,
};

#[derive(Clone, Debug)]
pub struct NetworkPathProfile {
    mode: NetworkStackMode,
    kernel_bypass_enabled: bool,
    kernel_bypass_engine: KernelBypassEngine,
    fpga_vendor: NonEmptyText,
}

impl NetworkPathProfile {
    pub fn from_settings(settings: &RuntimeSettings) -> Self {
        Self {
            mode: settings.network_stack_mode,
            kernel_bypass_enabled: settings.kernel_tcp_bypass_enabled,
            kernel_bypass_engine: settings.kernel_tcp_bypass_engine,
            fpga_vendor: settings.fpga_vendor.clone(),
        }
    }
}

impl NetworkPathPort for NetworkPathProfile {
    fn mode(&self) -> NetworkStackMode {
        self.mode
    }

    fn describe(&self) -> String {
        match self.mode {
            NetworkStackMode::Fpga => format!(
                "fpga path active via {} NIC (hardware timestamp and deterministic queue)",
                self.fpga_vendor.as_str()
            ),
            NetworkStackMode::KernelBypass => format!(
                "kernel tcp bypass active via {} (userspace packet path)",
                self.kernel_bypass_engine.as_str()
            ),
            NetworkStackMode::StandardTcp => {
                "kernel tcp bypass disabled (standard kernel socket path)".to_owned()
            }
        }
    }

    fn kernel_bypass_enabled(&self) -> bool {
        self.kernel_bypass_enabled
    }

    fn fpga_enabled(&self) -> bool {
        matches!(self.mode, NetworkStackMode::Fpga)
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkPathProfile;
    use crate::{
        domain::{
            settings::{NetworkStackMode, RuntimeSettings},
            value_objects::{
                KernelBypassEngine, NonEmptyText, PriorityFeesMicrolamports, ReplayBurstSize,
                ReplayEventCount, TxSubmissionMode,
            },
        },
        ports::network_path::NetworkPathPort,
    };

    fn settings_for(mode: NetworkStackMode) -> Result<RuntimeSettings, &'static str> {
        Ok(RuntimeSettings {
            config_path: "sniper.toml".to_owned(),
            priority_fees: PriorityFeesMicrolamports::new(1),
            keypair_path: "keypair.json".to_owned(),
            tx_submission_mode: TxSubmissionMode::Jito,
            jito_url: "https://jito.example".to_owned(),
            rpc_url: "https://rpc.example".to_owned(),
            wss_url: NonEmptyText::try_from("wss://wss.example".to_owned())?,
            kernel_tcp_bypass_enabled: true,
            kernel_tcp_bypass_engine: KernelBypassEngine::AfXdp,
            kernel_bypass_socket_path: NonEmptyText::try_from(
                "/tmp/sniper-kernel-bypass.sock".to_owned(),
            )?,
            fpga_enabled: mode == NetworkStackMode::Fpga,
            fpga_verbose: false,
            fpga_vendor: NonEmptyText::try_from("exanic".to_owned())?,
            network_stack_mode: mode,
            run_replay_benchmark: false,
            replay_event_count: ReplayEventCount::new(50_000)?,
            replay_burst_size: ReplayBurstSize::new(512)?,
            latency_sample_capacity: 4_096,
            latency_slo_ns: 1_000_000,
            latency_report_period_secs: 15,
            telemetry_enabled: true,
        })
    }

    #[test]
    fn fpga_mode_describes_vendor() {
        let settings = settings_for(NetworkStackMode::Fpga);
        assert!(settings.is_ok());
        let settings = if let Ok(settings) = settings {
            settings
        } else {
            return;
        };
        let profile = NetworkPathProfile::from_settings(&settings);

        assert!(profile.fpga_enabled());
        assert!(profile.describe().contains("exanic"));
    }

    #[test]
    fn standard_mode_disables_fpga() {
        let settings = settings_for(NetworkStackMode::StandardTcp);
        assert!(settings.is_ok());
        let settings = if let Ok(settings) = settings {
            settings
        } else {
            return;
        };
        let profile = NetworkPathProfile::from_settings(&settings);

        assert!(!profile.fpga_enabled());
        assert_eq!(profile.mode(), NetworkStackMode::StandardTcp);
    }
}
