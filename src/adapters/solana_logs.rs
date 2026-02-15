use std::{
    io::{BufRead, BufReader},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::{Path, PathBuf};

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

    fn validate_startup_ready(&self) -> Result<(), LogStreamError> {
        if !self.wss_url.starts_with("wss://") && !self.wss_url.starts_with("ws://") {
            return Err(LogStreamError::InvalidWebsocketUrl {
                url: self.wss_url.clone(),
                path: self.path_name,
            });
        }

        if self.source == IngressSource::KernelBypass && self.kernel_bypass_engine.is_none() {
            return Err(LogStreamError::MissingKernelBypassEngine);
        }

        if self.requires_openonload_runtime() && !openonload_runtime_ready() {
            return Err(LogStreamError::OpenOnloadRuntimeInactive);
        }

        if self.source == IngressSource::KernelBypass && self.prefers_external_bypass_feed() {
            self.validate_external_bypass_socket_ready()?;
        }

        Ok(())
    }

    const fn requires_openonload_runtime(&self) -> bool {
        matches!(self.source, IngressSource::KernelBypass)
            && matches!(
                self.kernel_bypass_engine,
                Some(KernelBypassEngine::OpenOnload)
            )
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

    #[cfg(unix)]
    fn validate_external_bypass_socket_ready(&self) -> Result<(), LogStreamError> {
        let socket_path = self.kernel_bypass_socket_path()?;
        let socket_path_ref = Path::new(&socket_path);
        if !socket_path_ref.exists() {
            return Err(LogStreamError::KernelBypassSocketUnavailable {
                socket_path: PathBuf::from(socket_path_ref),
            });
        }

        if UnixStream::connect(socket_path_ref).is_err() {
            return Err(LogStreamError::KernelBypassSocketUnavailable {
                socket_path: PathBuf::from(socket_path_ref),
            });
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn validate_external_bypass_socket_ready(&self) -> Result<(), LogStreamError> {
        Err(LogStreamError::ExternalBypassRequiresUnixTarget)
    }

    fn kernel_bypass_socket_path(&self) -> Result<String, LogStreamError> {
        self.kernel_bypass_socket_path
            .clone()
            .ok_or(LogStreamError::MissingKernelBypassSocketPath)
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
        let socket_path = self.kernel_bypass_socket_path()?;
        self.validate_external_bypass_socket_ready()?;

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
        Err(LogStreamError::ExternalBypassRequiresUnixTarget)
    }
}

impl LogStreamPort for SolanaPubsubLogStream {
    fn path_name(&self) -> &'static str {
        self.path_name
    }

    fn validate_ready(&self) -> Result<(), LogStreamError> {
        self.validate_startup_ready()
    }

    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError> {
        self.validate_startup_ready()?;

        if self.source == IngressSource::KernelBypass && self.prefers_external_bypass_feed() {
            return self.spawn_external_bypass_stream(sender);
        }

        self.spawn_websocket_stream(sender);
        Ok(())
    }
}

fn openonload_runtime_ready() -> bool {
    openonload_runtime_ready_with(openonload_device_available(), openonload_preload_active())
}

const fn openonload_runtime_ready_with(device_available: bool, preload_active: bool) -> bool {
    device_available && preload_active
}

fn openonload_device_available() -> bool {
    std::path::Path::new("/dev/onload").exists()
}

fn openonload_preload_active() -> bool {
    if let Ok(preload) = std::env::var("LD_PRELOAD") {
        let normalized = preload.to_ascii_lowercase();
        if normalized.contains("libonload") || normalized.contains("libcitransport") {
            return true;
        }
    }

    std::env::var("ONLOAD_PRELOAD").is_ok()
}

#[cfg(test)]
mod tests {
    use super::{SolanaPubsubLogStream, openonload_runtime_ready_with};
    use crate::domain::value_objects::KernelBypassEngine;
    use crate::ports::log_stream::{LogStreamError, LogStreamPort};
    use tokio::sync::mpsc;

    #[test]
    fn kernel_bypass_requires_valid_url() {
        let stream = SolanaPubsubLogStream::kernel_bypass(
            "invalid_url".to_owned(),
            KernelBypassEngine::AfXdp,
            "/tmp/slotstrike-kernel-bypass.sock".to_owned(),
        );
        let (sender, _receiver) = mpsc::unbounded_channel();

        let started = stream.spawn_stream(sender);
        assert!(started.is_err());
        if let Err(error) = started {
            assert!(matches!(error, LogStreamError::InvalidWebsocketUrl { .. }));
        }
    }

    #[test]
    fn path_name_matches_mode() {
        let kernel_stream = SolanaPubsubLogStream::kernel_bypass(
            "wss://example".to_owned(),
            KernelBypassEngine::AfXdp,
            "/tmp/slotstrike-kernel-bypass.sock".to_owned(),
        );
        let standard_stream = SolanaPubsubLogStream::standard_tcp("wss://example".to_owned());

        assert_eq!(kernel_stream.path_name(), "kernel_bypass");
        assert_eq!(standard_stream.path_name(), "standard_tcp");
    }

    #[test]
    fn openonload_kernel_bypass_startup_reflects_runtime_state() {
        let stream = SolanaPubsubLogStream::kernel_bypass(
            "wss://example".to_owned(),
            KernelBypassEngine::OpenOnload,
            "/tmp/slotstrike-kernel-bypass.sock".to_owned(),
        );
        let (sender, _receiver) = mpsc::unbounded_channel();

        let started = stream.spawn_stream(sender);
        if super::openonload_runtime_ready() {
            assert!(started.is_ok());
        } else {
            assert!(matches!(
                started,
                Err(LogStreamError::OpenOnloadRuntimeInactive)
            ));
        }
    }

    #[test]
    fn openonload_runtime_requires_device_and_preload() {
        assert!(!openonload_runtime_ready_with(false, false));
        assert!(!openonload_runtime_ready_with(true, false));
        assert!(!openonload_runtime_ready_with(false, true));
        assert!(openonload_runtime_ready_with(true, true));
    }
}
