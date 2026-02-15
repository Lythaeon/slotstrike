use std::path::PathBuf;

use thiserror::Error;
use tokio::sync::mpsc;

use crate::domain::events::RawLogEvent;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FpgaFeedError {
    #[error(
        "unsupported fpga_vendor '{vendor}'; supported values: mock_dma, generic, exanic, xilinx, amd, solarflare, napatech"
    )]
    UnsupportedVendor { vendor: String },
    #[error("mock FPGA DMA ring requires '{env_var}' environment payload")]
    MissingMockPayloadEnv { env_var: &'static str },
    #[error("configured FPGA DMA socket path '{socket_path}' does not exist")]
    DmaSocketPathMissing { socket_path: PathBuf },
    #[error("failed to connect FPGA DMA socket at '{socket_path}'")]
    DmaSocketUnavailable { socket_path: PathBuf },
    #[error("FPGA external DMA socket mode requires unix target")]
    ExternalSocketRequiresUnixTarget,
    #[error("configured FPGA direct device path '{device_path}' does not exist")]
    DirectDevicePathMissing { device_path: PathBuf },
    #[error("failed to open FPGA direct device at '{device_path}'")]
    DirectDeviceUnavailable { device_path: PathBuf },
    #[error("FPGA direct device mode requires unix target")]
    DirectDeviceRequiresUnixTarget,
    #[error("FPGA external frame is not valid JSON")]
    ExternalFrameInvalidJson,
    #[error("FPGA external frame must include either payload or payload_base64")]
    ExternalFrameMissingPayload,
    #[error("FPGA external frame payload_base64 is invalid")]
    ExternalFrameInvalidBase64,
    #[error("FPGA DMA payload is not valid UTF-8")]
    InvalidPayloadUtf8,
    #[error("FPGA DMA frame contains empty signature")]
    EmptySignature,
    #[error("FPGA DMA frame has invalid has_error flag")]
    InvalidHasErrorFlag,
    #[error("FPGA DMA frame missing signature field")]
    MissingSignature,
    #[error("FPGA DMA frame does not contain logs")]
    MissingLogs,
}

pub trait FpgaFeedPort: Send + Sync {
    fn vendor(&self) -> &str;
    fn verbose(&self) -> bool;
    fn describe(&self) -> String;
    fn validate_ready(&self) -> Result<(), FpgaFeedError>;
    fn spawn_stream(&self, sender: mpsc::UnboundedSender<RawLogEvent>)
    -> Result<(), FpgaFeedError>;
}
