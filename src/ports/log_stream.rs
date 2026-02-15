use std::path::PathBuf;

use thiserror::Error;
use tokio::sync::mpsc;

use crate::domain::events::RawLogEvent;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum LogStreamError {
    #[error("{0}")]
    Unavailable(String),
    #[error("invalid websocket url '{url}' for {path} path")]
    InvalidWebsocketUrl { url: String, path: &'static str },
    #[error("kernel bypass path missing engine selection")]
    MissingKernelBypassEngine,
    #[error(
        "openonload engine selected but Onload runtime is inactive; ensure /dev/onload is present and launch via onload preload"
    )]
    OpenOnloadRuntimeInactive,
    #[error("missing kernel bypass socket path")]
    MissingKernelBypassSocketPath,
    #[error(
        "kernel bypass socket unavailable at '{socket_path}' (expected external AF_XDP/DPDK bridge)"
    )]
    KernelBypassSocketUnavailable { socket_path: PathBuf },
    #[error("kernel bypass external socket mode requires unix target")]
    ExternalBypassRequiresUnixTarget,
}

pub trait LogStreamPort: Send + Sync {
    fn path_name(&self) -> &'static str;
    fn validate_ready(&self) -> Result<(), LogStreamError>;
    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError>;
}
