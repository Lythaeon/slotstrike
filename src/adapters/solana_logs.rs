use std::{
    io::{BufRead, BufReader},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use serde::Deserialize;
use solana_client::{
    pubsub_client::PubsubClient,
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_commitment_config::CommitmentConfig;
use tokio::sync::mpsc;

use crate::{
    domain::events::{IngressMetadata, IngressSource, RawLogEvent, unix_timestamp_now_ns},
    domain::value_objects::KernelBypassEngine,
    ports::log_stream::{LogStreamError, LogStreamPort},
};

#[derive(Clone, Debug)]
pub struct SolanaPubsubLogStream {
    pub wss_url: String,
    path_name: &'static str,
    source: IngressSource,
    kernel_bypass_engine: Option<KernelBypassEngine>,
    kernel_bypass_socket_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ExternalBypassEvent {
    signature: String,
    logs: Vec<String>,
    #[serde(default)]
    has_error: bool,
    #[serde(default)]
    hardware_timestamp_ns: Option<u64>,
    #[serde(default)]
    received_timestamp_ns: Option<u64>,
}

impl SolanaPubsubLogStream {
    pub const fn kernel_bypass(
        wss_url: String,
        kernel_bypass_engine: KernelBypassEngine,
        kernel_bypass_socket_path: String,
    ) -> Self {
        Self {
            wss_url,
            path_name: "kernel_bypass",
            source: IngressSource::KernelBypass,
            kernel_bypass_engine: Some(kernel_bypass_engine),
            kernel_bypass_socket_path: Some(kernel_bypass_socket_path),
        }
    }

    pub const fn standard_tcp(wss_url: String) -> Self {
        Self {
            wss_url,
            path_name: "standard_tcp",
            source: IngressSource::StandardTcp,
            kernel_bypass_engine: None,
            kernel_bypass_socket_path: None,
        }
    }

    fn validate_ready(&self) -> Result<(), LogStreamError> {
        if !self.wss_url.starts_with("wss://") && !self.wss_url.starts_with("ws://") {
            return Err(LogStreamError::Unavailable(format!(
                "invalid websocket url '{}' for {} path",
                self.wss_url, self.path_name
            )));
        }

        if self.source == IngressSource::KernelBypass && self.kernel_bypass_engine.is_none() {
            return Err(LogStreamError::Unavailable(
                "kernel bypass path missing engine selection".to_owned(),
            ));
        }

        Ok(())
    }

    const fn prefers_external_bypass_feed(&self) -> bool {
        matches!(
            self.kernel_bypass_engine,
            Some(
                KernelBypassEngine::AfXdp
                    | KernelBypassEngine::Dpdk
                    | KernelBypassEngine::AfXdpOrDpdkExternal
            )
        )
    }

    fn spawn_websocket_stream(&self, sender: mpsc::UnboundedSender<RawLogEvent>) {
        let wss_url = self.wss_url.clone();
        let source = self.source;
        let path_name = self.path_name;

        thread::spawn(move || {
            loop {
                let logs_filter = RpcTransactionLogsFilter::All;
                let logs_config = RpcTransactionLogsConfig {
                    commitment: Some(CommitmentConfig::confirmed()),
                };

                let ws = match PubsubClient::logs_subscribe(&wss_url, logs_filter, logs_config) {
                    Ok(value) => value,
                    Err(error) => {
                        log::error!("Log subscription failed: {}", error);
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };

                log::info!("Listening for tokens on {} path...", path_name);

                while let Ok(response) = ws.1.recv_timeout(Duration::from_secs(30)) {
                    let received_timestamp_ns = unix_timestamp_now_ns();
                    let event = RawLogEvent {
                        signature: response.value.signature,
                        logs: response.value.logs,
                        has_error: response.value.err.is_some(),
                        ingress: IngressMetadata::from_receive_clock(source, received_timestamp_ns),
                    };

                    if sender.send(event).is_err() {
                        log::warn!("Event channel closed. Stopping log stream.");
                        return;
                    }
                }

                if let Err(error) = ws.0.send_unsubscribe() {
                    log::debug!("Failed to unsubscribe websocket stream: {}", error);
                }
                log::warn!("Connection lost, attempting to reconnect in 5 seconds...");
                thread::sleep(Duration::from_secs(5));
            }
        });
    }

    #[cfg(unix)]
    fn spawn_external_bypass_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError> {
        let socket_path = self.kernel_bypass_socket_path.clone().ok_or_else(|| {
            LogStreamError::Unavailable("Missing kernel bypass socket path".to_owned())
        })?;

        if UnixStream::connect(&socket_path).is_err() {
            return Err(LogStreamError::Unavailable(format!(
                "kernel bypass socket unavailable at '{}' (expected external AF_XDP/DPDK bridge)",
                socket_path
            )));
        }

        thread::spawn(move || {
            loop {
                let stream = match UnixStream::connect(&socket_path) {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!(
                            "Kernel bypass socket reconnect failed for '{}': {}",
                            socket_path,
                            error
                        );
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };

                log::info!(
                    "Listening for tokens on kernel_bypass path via external socket {}",
                    socket_path
                );

                let mut reader = BufReader::new(stream);

                loop {
                    let mut frame = String::new();
                    let read_len = match reader.read_line(&mut frame) {
                        Ok(value) => value,
                        Err(error) => {
                            log::warn!("Kernel bypass socket read failed: {}", error);
                            break;
                        }
                    };

                    if read_len == 0 {
                        log::warn!("Kernel bypass socket closed by peer. Reconnecting.");
                        break;
                    }

                    let payload = frame.trim();
                    if payload.is_empty() {
                        continue;
                    }

                    let parsed = match serde_json::from_str::<ExternalBypassEvent>(payload) {
                        Ok(value) => value,
                        Err(error) => {
                            log::debug!("Kernel bypass frame parse failed: {}", error);
                            continue;
                        }
                    };

                    let received_timestamp_ns = parsed
                        .received_timestamp_ns
                        .unwrap_or_else(unix_timestamp_now_ns);
                    let event = RawLogEvent {
                        signature: parsed.signature,
                        logs: parsed.logs,
                        has_error: parsed.has_error,
                        ingress: IngressMetadata::from_hardware_clock(
                            IngressSource::KernelBypass,
                            parsed.hardware_timestamp_ns,
                            received_timestamp_ns,
                        ),
                    };

                    if sender.send(event).is_err() {
                        log::warn!("Event channel closed. Stopping kernel bypass stream.");
                        return;
                    }
                }

                thread::sleep(Duration::from_millis(250));
            }
        });

        Ok(())
    }

    #[cfg(not(unix))]
    fn spawn_external_bypass_stream(
        &self,
        _sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError> {
        Err(LogStreamError::Unavailable(
            "kernel bypass external socket mode requires unix target".to_owned(),
        ))
    }
}

impl LogStreamPort for SolanaPubsubLogStream {
    fn path_name(&self) -> &'static str {
        self.path_name
    }

    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError> {
        self.validate_ready()?;

        if self.source == IngressSource::KernelBypass && self.prefers_external_bypass_feed() {
            return self.spawn_external_bypass_stream(sender);
        }

        self.spawn_websocket_stream(sender);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SolanaPubsubLogStream;
    use crate::domain::value_objects::KernelBypassEngine;
    use crate::ports::log_stream::{LogStreamError, LogStreamPort};
    use tokio::sync::mpsc;

    #[test]
    fn kernel_bypass_requires_valid_url() {
        let stream = SolanaPubsubLogStream::kernel_bypass(
            "invalid_url".to_owned(),
            KernelBypassEngine::AfXdp,
            "/tmp/sniper-kernel-bypass.sock".to_owned(),
        );
        let (sender, _receiver) = mpsc::unbounded_channel();

        let started = stream.spawn_stream(sender);
        assert!(started.is_err());
        if let Err(error) = started {
            assert!(matches!(error, LogStreamError::Unavailable(_)));
        }
    }

    #[test]
    fn path_name_matches_mode() {
        let kernel_stream = SolanaPubsubLogStream::kernel_bypass(
            "wss://example".to_owned(),
            KernelBypassEngine::AfXdp,
            "/tmp/sniper-kernel-bypass.sock".to_owned(),
        );
        let standard_stream = SolanaPubsubLogStream::standard_tcp("wss://example".to_owned());

        assert_eq!(kernel_stream.path_name(), "kernel_bypass");
        assert_eq!(standard_stream.path_name(), "standard_tcp");
    }
}
