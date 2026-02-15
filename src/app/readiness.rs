use crate::{
    app::errors::IngressReadinessError,
    ports::{fpga_feed::FpgaFeedPort, log_stream::LogStreamPort, network_path::NetworkPathPort},
};

pub fn validate_ingress_readiness(
    network_path: &dyn NetworkPathPort,
    fpga_feed: &dyn FpgaFeedPort,
    kernel_bypass_stream: &dyn LogStreamPort,
    standard_tcp_stream: &dyn LogStreamPort,
) -> Result<(), IngressReadinessError> {
    match network_path.mode() {
        crate::domain::settings::NetworkStackMode::Fpga => fpga_feed
            .validate_ready()
            .map_err(|source| IngressReadinessError::Fpga { source }),
        crate::domain::settings::NetworkStackMode::KernelBypass => kernel_bypass_stream
            .validate_ready()
            .map_err(|source| IngressReadinessError::KernelBypass { source }),
        crate::domain::settings::NetworkStackMode::StandardTcp => standard_tcp_stream
            .validate_ready()
            .map_err(|source| IngressReadinessError::StandardTcp { source }),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::readiness::validate_ingress_readiness,
        domain::{events::RawLogEvent, settings::NetworkStackMode},
        ports::{
            fpga_feed::{FpgaFeedError, FpgaFeedPort},
            log_stream::{LogStreamError, LogStreamPort},
            network_path::NetworkPathPort,
        },
    };
    use tokio::sync::mpsc;

    #[derive(Clone, Debug)]
    struct FakeNetworkPath {
        mode: NetworkStackMode,
    }

    impl NetworkPathPort for FakeNetworkPath {
        fn mode(&self) -> NetworkStackMode {
            self.mode
        }

        fn describe(&self) -> String {
            "fake".to_owned()
        }

        fn kernel_bypass_enabled(&self) -> bool {
            false
        }

        fn fpga_enabled(&self) -> bool {
            self.mode == NetworkStackMode::Fpga
        }
    }

    #[derive(Clone, Debug)]
    struct FakeFpgaFeed {
        ready: Result<(), FpgaFeedError>,
    }

    impl FpgaFeedPort for FakeFpgaFeed {
        fn vendor(&self) -> &str {
            "fake"
        }

        fn verbose(&self) -> bool {
            false
        }

        fn describe(&self) -> String {
            "fake".to_owned()
        }

        fn validate_ready(&self) -> Result<(), FpgaFeedError> {
            self.ready.clone()
        }

        fn spawn_stream(
            &self,
            _sender: mpsc::UnboundedSender<RawLogEvent>,
        ) -> Result<(), FpgaFeedError> {
            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    struct FakeLogStream {
        ready: Result<(), LogStreamError>,
    }

    impl LogStreamPort for FakeLogStream {
        fn path_name(&self) -> &'static str {
            "fake"
        }

        fn validate_ready(&self) -> Result<(), LogStreamError> {
            self.ready.clone()
        }

        fn spawn_stream(
            &self,
            _sender: mpsc::UnboundedSender<RawLogEvent>,
        ) -> Result<(), LogStreamError> {
            Ok(())
        }
    }

    #[test]
    fn fpga_mode_returns_fpga_readiness_error() {
        let network_path = FakeNetworkPath {
            mode: NetworkStackMode::Fpga,
        };
        let fpga_feed = FakeFpgaFeed {
            ready: Err(FpgaFeedError::MissingMockPayloadEnv {
                env_var: "FPGA_DMA_MOCK_FRAME",
            }),
        };
        let kernel_bypass_stream = FakeLogStream { ready: Ok(()) };
        let standard_tcp_stream = FakeLogStream { ready: Ok(()) };

        let ready = validate_ingress_readiness(
            &network_path,
            &fpga_feed,
            &kernel_bypass_stream,
            &standard_tcp_stream,
        );
        assert!(ready.is_err());
        if let Err(error) = ready {
            assert!(matches!(
                error,
                crate::app::errors::IngressReadinessError::Fpga { .. }
            ));
        }
    }

    #[test]
    fn kernel_bypass_mode_returns_kernel_readiness_error() {
        let network_path = FakeNetworkPath {
            mode: NetworkStackMode::KernelBypass,
        };
        let fpga_feed = FakeFpgaFeed { ready: Ok(()) };
        let kernel_bypass_stream = FakeLogStream {
            ready: Err(LogStreamError::OpenOnloadRuntimeInactive),
        };
        let standard_tcp_stream = FakeLogStream { ready: Ok(()) };

        let ready = validate_ingress_readiness(
            &network_path,
            &fpga_feed,
            &kernel_bypass_stream,
            &standard_tcp_stream,
        );
        assert!(ready.is_err());
        if let Err(error) = ready {
            assert!(matches!(
                error,
                crate::app::errors::IngressReadinessError::KernelBypass { .. }
            ));
        }
    }
}
