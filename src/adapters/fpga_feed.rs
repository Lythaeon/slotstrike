use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    sync::Arc,
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::{
    os::unix::{fs::FileTypeExt, net::UnixStream},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::{
    domain::events::{
        IngressMetadata, IngressSource, RawLogEvent, normalize_hardware_timestamp_ns,
        unix_timestamp_now_ns,
    },
    domain::value_objects::FpgaIngressMode,
    ports::fpga_feed::{FpgaFeedError, FpgaFeedPort},
    slices::sniper::pool_filter::is_pool_creation_dma_payload,
};

const MOCK_DMA_VENDOR: &str = "mock_dma";
const MOCK_DMA_FRAME_ENV: &str = "FPGA_DMA_MOCK_FRAME";
const DEFAULT_FPGA_DMA_SOCKET_PATH: &str = "/tmp/slotstrike-fpga-dma.sock";
const DEFAULT_FPGA_DIRECT_DEVICE_PATH: &str = "/dev/slotstrike-fpga0";
const EXTERNAL_DMA_VENDORS: [&str; 6] = [
    "generic",
    "exanic",
    "xilinx",
    "amd",
    "solarflare",
    "napatech",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FpgaIngressBackend {
    MockDmaRing,
    DirectDevice,
    ExternalSocket,
}

#[derive(Clone, Debug)]
struct DmaFrame {
    hardware_timestamp_ns: u64,
    payload: Arc<[u8]>,
}

impl DmaFrame {
    #[inline(always)]
    const fn new(hardware_timestamp_ns: u64, payload: Arc<[u8]>) -> Self {
        Self {
            hardware_timestamp_ns,
            payload,
        }
    }

    #[inline(always)]
    const fn hardware_timestamp_ns(&self) -> u64 {
        self.hardware_timestamp_ns
    }

    #[inline(always)]
    fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug)]
struct DmaRing {
    queue: VecDeque<DmaFrame>,
}

impl DmaRing {
    fn with_capacity(slot_count: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(slot_count),
        }
    }

    fn push(&mut self, frame: DmaFrame) {
        self.queue.push_back(frame);
    }

    fn pop(&mut self) -> Option<DmaFrame> {
        self.queue.pop_front()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct ExternalDmaFrame {
    #[serde(default)]
    hardware_timestamp_ns: Option<u64>,
    #[serde(default)]
    payload: Option<String>,
    #[serde(default)]
    payload_base64: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedDmaPayload {
    signature: String,
    logs: Vec<String>,
    has_error: bool,
}

impl DecodedDmaPayload {
    #[inline(always)]
    pub fn signature(&self) -> &str {
        &self.signature
    }

    #[inline(always)]
    pub fn logs(&self) -> &[String] {
        &self.logs
    }

    #[inline(always)]
    pub const fn has_error(&self) -> bool {
        self.has_error
    }
}

#[derive(Clone, Debug)]
pub struct FpgaFeedAdapter {
    vendor: String,
    verbose: bool,
    ingress_mode: FpgaIngressMode,
    direct_device_path: String,
    dma_socket_path: String,
    mock_dma_payload: Option<Arc<[u8]>>,
}

impl FpgaFeedAdapter {
    pub fn new(vendor: String, verbose: bool) -> Self {
        Self {
            vendor,
            verbose,
            ingress_mode: FpgaIngressMode::Auto,
            direct_device_path: DEFAULT_FPGA_DIRECT_DEVICE_PATH.to_owned(),
            dma_socket_path: DEFAULT_FPGA_DMA_SOCKET_PATH.to_owned(),
            mock_dma_payload: None,
        }
    }

    pub const fn with_ingress_mode(mut self, ingress_mode: FpgaIngressMode) -> Self {
        self.ingress_mode = ingress_mode;
        self
    }

    pub fn with_direct_device_path(mut self, direct_device_path: String) -> Self {
        self.direct_device_path = direct_device_path;
        self
    }

    pub fn with_dma_socket_path(mut self, dma_socket_path: String) -> Self {
        self.dma_socket_path = dma_socket_path;
        self
    }

    pub fn dma_socket_path(&self) -> &str {
        &self.dma_socket_path
    }

    pub fn direct_device_path(&self) -> &str {
        &self.direct_device_path
    }

    fn ingress_backend(&self) -> Result<FpgaIngressBackend, FpgaFeedError> {
        match self.ingress_mode {
            FpgaIngressMode::Auto => {
                if self.vendor.eq_ignore_ascii_case(MOCK_DMA_VENDOR) {
                    return Ok(FpgaIngressBackend::MockDmaRing);
                }

                if is_direct_dma_vendor(&self.vendor) {
                    return Ok(FpgaIngressBackend::DirectDevice);
                }

                Err(FpgaFeedError::UnsupportedVendor {
                    vendor: self.vendor.clone(),
                })
            }
            FpgaIngressMode::MockDma => Ok(FpgaIngressBackend::MockDmaRing),
            FpgaIngressMode::DirectDevice => {
                if is_direct_dma_vendor(&self.vendor) {
                    Ok(FpgaIngressBackend::DirectDevice)
                } else {
                    Err(FpgaFeedError::UnsupportedVendor {
                        vendor: self.vendor.clone(),
                    })
                }
            }
            FpgaIngressMode::ExternalSocket => {
                if is_external_dma_vendor(&self.vendor) {
                    Ok(FpgaIngressBackend::ExternalSocket)
                } else {
                    Err(FpgaFeedError::UnsupportedVendor {
                        vendor: self.vendor.clone(),
                    })
                }
            }
        }
    }

    fn validate_mock_ring_ready(&self) -> Result<(), FpgaFeedError> {
        if self.resolve_mock_dma_payload().is_none() {
            return Err(FpgaFeedError::MissingMockPayloadEnv {
                env_var: MOCK_DMA_FRAME_ENV,
            });
        }

        Ok(())
    }

    #[cfg(unix)]
    fn validate_direct_device_ready(&self) -> Result<(), FpgaFeedError> {
        let device_path = Path::new(&self.direct_device_path);
        if !device_path.exists() {
            return Err(FpgaFeedError::DirectDevicePathMissing {
                device_path: PathBuf::from(device_path),
            });
        }

        let metadata = std::fs::metadata(device_path).map_err(|_source| {
            FpgaFeedError::DirectDevicePathMissing {
                device_path: PathBuf::from(device_path),
            }
        })?;
        let file_type = metadata.file_type();

        if !(file_type.is_char_device() || file_type.is_file() || file_type.is_fifo()) {
            return Err(FpgaFeedError::DirectDeviceUnavailable {
                device_path: PathBuf::from(device_path),
            });
        }

        if std::fs::File::open(device_path).is_err() {
            return Err(FpgaFeedError::DirectDeviceUnavailable {
                device_path: PathBuf::from(device_path),
            });
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn validate_direct_device_ready(&self) -> Result<(), FpgaFeedError> {
        Err(FpgaFeedError::DirectDeviceRequiresUnixTarget)
    }

    #[cfg(unix)]
    fn validate_external_socket_ready(&self) -> Result<(), FpgaFeedError> {
        let socket_path = Path::new(&self.dma_socket_path);
        if !socket_path.exists() {
            return Err(FpgaFeedError::DmaSocketPathMissing {
                socket_path: PathBuf::from(socket_path),
            });
        }

        let metadata = std::fs::metadata(socket_path).map_err(|_source| {
            FpgaFeedError::DmaSocketPathMissing {
                socket_path: PathBuf::from(socket_path),
            }
        })?;

        if !metadata.file_type().is_socket() {
            return Err(FpgaFeedError::DmaSocketUnavailable {
                socket_path: PathBuf::from(socket_path),
            });
        }

        if UnixStream::connect(socket_path).is_err() {
            return Err(FpgaFeedError::DmaSocketUnavailable {
                socket_path: PathBuf::from(socket_path),
            });
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn validate_external_socket_ready(&self) -> Result<(), FpgaFeedError> {
        Err(FpgaFeedError::ExternalSocketRequiresUnixTarget)
    }

    fn bootstrap_dma_ring(&self) -> Result<DmaRing, FpgaFeedError> {
        self.validate_mock_ring_ready()?;

        let payload =
            self.resolve_mock_dma_payload()
                .ok_or(FpgaFeedError::MissingMockPayloadEnv {
                    env_var: MOCK_DMA_FRAME_ENV,
                })?;

        let frame = DmaFrame::new(unix_timestamp_now_ns(), payload);
        let mut ring = DmaRing::with_capacity(1_024);
        ring.push(frame);
        Ok(ring)
    }

    fn resolve_mock_dma_payload(&self) -> Option<Arc<[u8]>> {
        self.mock_dma_payload.as_ref().map(Arc::clone).or_else(|| {
            std::env::var(MOCK_DMA_FRAME_ENV)
                .ok()
                .map(|payload| Arc::<[u8]>::from(payload.into_bytes()))
        })
    }

    fn decode_dma_frame(frame: &DmaFrame) -> Result<DecodedDmaPayload, FpgaFeedError> {
        decode_dma_payload(frame.payload())
    }

    fn process_frame(
        frame: &DmaFrame,
        sender: &mpsc::UnboundedSender<RawLogEvent>,
        verbose: bool,
    ) -> bool {
        if verbose {
            log::debug!(
                "FPGA DMA RX > ts={} ns, bytes={}",
                frame.hardware_timestamp_ns(),
                frame.payload().len()
            );
        }

        if !is_pool_creation_dma_payload(frame.payload()) {
            if verbose {
                log::debug!("FPGA DMA RX > frame skipped by deterministic prefilter");
            }
            return true;
        }

        let parsed = match Self::decode_dma_frame(frame) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("FPGA DMA decode failed: {}", error);
                return true;
            }
        };

        let received_timestamp_ns = unix_timestamp_now_ns();
        let normalized_timestamp_ns = normalize_hardware_timestamp_ns(
            Some(frame.hardware_timestamp_ns()),
            received_timestamp_ns,
        );
        let event = RawLogEvent {
            signature: parsed.signature,
            logs: parsed.logs,
            has_error: parsed.has_error,
            ingress: IngressMetadata {
                source: IngressSource::FpgaDma,
                hardware_timestamp_ns: Some(frame.hardware_timestamp_ns()),
                received_timestamp_ns,
                normalized_timestamp_ns,
            },
        };

        if sender.send(event).is_err() {
            log::warn!("FPGA event channel closed. Stopping DMA stream.");
            return false;
        }

        true
    }

    #[cfg(unix)]
    fn parse_external_frame(raw_frame: &str) -> Result<DmaFrame, FpgaFeedError> {
        let parsed = serde_json::from_str::<ExternalDmaFrame>(raw_frame)
            .map_err(|_source| FpgaFeedError::ExternalFrameInvalidJson)?;

        let payload = if let Some(payload_base64) = parsed.payload_base64 {
            BASE64_STANDARD
                .decode(payload_base64.as_bytes())
                .map_err(|_source| FpgaFeedError::ExternalFrameInvalidBase64)?
        } else if let Some(payload) = parsed.payload {
            payload.into_bytes()
        } else {
            return Err(FpgaFeedError::ExternalFrameMissingPayload);
        };

        if payload.is_empty() {
            return Err(FpgaFeedError::ExternalFrameMissingPayload);
        }

        let hardware_timestamp_ns = parsed
            .hardware_timestamp_ns
            .unwrap_or_else(unix_timestamp_now_ns);
        Ok(DmaFrame::new(
            hardware_timestamp_ns,
            Arc::<[u8]>::from(payload),
        ))
    }

    #[cfg(unix)]
    fn parse_wire_frame(raw_frame: &str) -> Result<DmaFrame, FpgaFeedError> {
        if raw_frame.starts_with('{') {
            return Self::parse_external_frame(raw_frame);
        }

        if let Ok(payload) = BASE64_STANDARD.decode(raw_frame.as_bytes())
            && !payload.is_empty()
        {
            return Ok(DmaFrame::new(
                unix_timestamp_now_ns(),
                Arc::<[u8]>::from(payload),
            ));
        }

        Ok(DmaFrame::new(
            unix_timestamp_now_ns(),
            Arc::<[u8]>::from(raw_frame.as_bytes()),
        ))
    }

    #[cfg(unix)]
    fn spawn_external_socket_stream(&self, sender: mpsc::UnboundedSender<RawLogEvent>) {
        let socket_path = self.dma_socket_path.clone();
        let verbose = self.verbose;

        thread::spawn(move || {
            loop {
                let stream = match UnixStream::connect(Path::new(&socket_path)) {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!(
                            "FPGA DMA socket reconnect failed for '{}': {}",
                            socket_path,
                            error
                        );
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };

                log::info!(
                    "Listening for FPGA DMA frames via external socket {}",
                    socket_path
                );

                let mut reader = BufReader::new(stream);
                loop {
                    let mut raw_frame = String::new();
                    let read_len = match reader.read_line(&mut raw_frame) {
                        Ok(value) => value,
                        Err(error) => {
                            log::warn!("FPGA DMA socket read failed: {}", error);
                            break;
                        }
                    };

                    if read_len == 0 {
                        log::warn!("FPGA DMA socket closed by peer. Reconnecting.");
                        break;
                    }

                    let payload = raw_frame.trim();
                    if payload.is_empty() {
                        continue;
                    }

                    let frame = match Self::parse_external_frame(payload) {
                        Ok(value) => value,
                        Err(error) => {
                            log::debug!("FPGA external frame parse failed: {}", error);
                            continue;
                        }
                    };

                    if !Self::process_frame(&frame, &sender, verbose) {
                        return;
                    }
                }

                thread::sleep(Duration::from_millis(250));
            }
        });
    }

    #[cfg(unix)]
    fn spawn_direct_device_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), FpgaFeedError> {
        self.validate_direct_device_ready()?;

        let device_path = self.direct_device_path.clone();
        let verbose = self.verbose;

        thread::spawn(move || {
            loop {
                let device = match std::fs::File::open(Path::new(&device_path)) {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!(
                            "FPGA direct device open failed for '{}': {}",
                            device_path,
                            error
                        );
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };

                log::info!("Reading FPGA DMA frames from direct device {}", device_path);

                let mut reader = BufReader::new(device);
                loop {
                    let mut raw_frame = String::new();
                    let read_len = match reader.read_line(&mut raw_frame) {
                        Ok(value) => value,
                        Err(error) => {
                            log::warn!("FPGA direct device read failed: {}", error);
                            break;
                        }
                    };

                    if read_len == 0 {
                        log::warn!("FPGA direct device reached EOF. Reopening.");
                        break;
                    }

                    let payload = raw_frame.trim();
                    if payload.is_empty() {
                        continue;
                    }

                    let frame = match Self::parse_wire_frame(payload) {
                        Ok(value) => value,
                        Err(error) => {
                            log::debug!("FPGA direct frame parse failed: {}", error);
                            continue;
                        }
                    };

                    if !Self::process_frame(&frame, &sender, verbose) {
                        return;
                    }
                }

                thread::sleep(Duration::from_millis(250));
            }
        });

        Ok(())
    }

    #[cfg(not(unix))]
    fn spawn_direct_device_stream(
        &self,
        _sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), FpgaFeedError> {
        Err(FpgaFeedError::DirectDeviceRequiresUnixTarget)
    }

    #[cfg(not(unix))]
    fn spawn_external_socket_stream(&self, _sender: mpsc::UnboundedSender<RawLogEvent>) {
        log::error!("{}", FpgaFeedError::ExternalSocketRequiresUnixTarget);
    }

    pub fn with_mock_dma_payload(vendor: String, verbose: bool, payload: &[u8]) -> Self {
        Self {
            vendor,
            verbose,
            ingress_mode: FpgaIngressMode::MockDma,
            direct_device_path: DEFAULT_FPGA_DIRECT_DEVICE_PATH.to_owned(),
            dma_socket_path: DEFAULT_FPGA_DMA_SOCKET_PATH.to_owned(),
            mock_dma_payload: Some(Arc::<[u8]>::from(payload)),
        }
    }

    #[cfg(test)]
    fn with_inline_dma_payload(vendor: String, verbose: bool, payload: &[u8]) -> Self {
        Self::with_mock_dma_payload(vendor, verbose, payload)
    }
}

impl FpgaFeedPort for FpgaFeedAdapter {
    fn vendor(&self) -> &str {
        &self.vendor
    }

    fn verbose(&self) -> bool {
        self.verbose
    }

    fn describe(&self) -> String {
        if self.verbose {
            format!(
                "FPGA feed enabled (vendor={}, mode={}, verbose=true, hardware timestamps active, direct_device_path={}, dma_socket_path={})",
                self.vendor, self.ingress_mode, self.direct_device_path, self.dma_socket_path
            )
        } else {
            format!(
                "FPGA feed enabled (vendor={}, mode={}, verbose=false, direct_device_path={}, dma_socket_path={})",
                self.vendor, self.ingress_mode, self.direct_device_path, self.dma_socket_path
            )
        }
    }

    fn validate_ready(&self) -> Result<(), FpgaFeedError> {
        match self.ingress_backend()? {
            FpgaIngressBackend::MockDmaRing => self.validate_mock_ring_ready(),
            FpgaIngressBackend::DirectDevice => self.validate_direct_device_ready(),
            FpgaIngressBackend::ExternalSocket => self.validate_external_socket_ready(),
        }
    }

    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), FpgaFeedError> {
        self.validate_ready()?;

        match self.ingress_backend()? {
            FpgaIngressBackend::MockDmaRing => {
                let mut dma_ring = self.bootstrap_dma_ring()?;
                let verbose = self.verbose;

                thread::spawn(move || {
                    while let Some(frame) = dma_ring.pop() {
                        if !Self::process_frame(&frame, &sender, verbose) {
                            return;
                        }
                    }
                });

                Ok(())
            }
            FpgaIngressBackend::DirectDevice => self.spawn_direct_device_stream(sender),
            FpgaIngressBackend::ExternalSocket => {
                self.spawn_external_socket_stream(sender);
                Ok(())
            }
        }
    }
}

pub fn decode_dma_payload(payload: &[u8]) -> Result<DecodedDmaPayload, FpgaFeedError> {
    let payload =
        std::str::from_utf8(payload).map_err(|_parse_error| FpgaFeedError::InvalidPayloadUtf8)?;

    let mut signature: Option<String> = None;
    let mut logs = Vec::new();
    let mut has_error = false;

    for line in payload.lines() {
        if let Some(value) = line.strip_prefix("signature=") {
            if value.trim().is_empty() {
                return Err(FpgaFeedError::EmptySignature);
            }

            signature = Some(value.trim().to_owned());
            continue;
        }

        if let Some(value) = line.strip_prefix("has_error=") {
            has_error = parse_bool_flag(value).ok_or(FpgaFeedError::InvalidHasErrorFlag)?;
            continue;
        }

        if let Some(value) = line.strip_prefix("log=") {
            logs.push(value.to_owned());
        }
    }

    let signature = signature.ok_or(FpgaFeedError::MissingSignature)?;

    if logs.is_empty() {
        return Err(FpgaFeedError::MissingLogs);
    }

    Ok(DecodedDmaPayload {
        signature,
        logs,
        has_error,
    })
}

fn parse_bool_flag(value: &str) -> Option<bool> {
    let trimmed = value.trim();

    if trimmed == "1"
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("on")
    {
        return Some(true);
    }

    if trimmed == "0"
        || trimmed.eq_ignore_ascii_case("false")
        || trimmed.eq_ignore_ascii_case("no")
        || trimmed.eq_ignore_ascii_case("off")
    {
        return Some(false);
    }

    None
}

fn is_external_dma_vendor(vendor: &str) -> bool {
    EXTERNAL_DMA_VENDORS
        .iter()
        .any(|supported_vendor| vendor.eq_ignore_ascii_case(supported_vendor))
}

fn is_direct_dma_vendor(vendor: &str) -> bool {
    EXTERNAL_DMA_VENDORS
        .iter()
        .any(|supported_vendor| vendor.eq_ignore_ascii_case(supported_vendor))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    #[cfg(unix)]
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        path::PathBuf,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::adapters::raydium::{RAYDIUM_STANDARD_AMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID};
    use crate::ports::fpga_feed::FpgaFeedError;
    use tokio::sync::mpsc;

    #[test]
    fn fpga_description_contains_vendor() {
        let adapter = FpgaFeedAdapter::new("exanic".to_owned(), true);
        assert!(adapter.describe().contains("exanic"));
        assert!(adapter.verbose());
        assert_eq!(adapter.vendor(), "exanic");
        assert!(adapter.describe().contains(adapter.direct_device_path()));
        assert!(adapter.describe().contains(adapter.dma_socket_path()));
    }

    #[test]
    fn dma_frame_parser_builds_strategy_event() {
        let frame = DmaFrame::new(
            42,
            Arc::<[u8]>::from(
                b"signature=5M6A9\nhas_error=0\nlog=Program log: initialize2\nlog=Program log: init_pc_amount: 1,"
                    .as_slice(),
            ),
        );

        let parsed = FpgaFeedAdapter::decode_dma_frame(&frame);
        assert!(parsed.is_ok());

        if let Ok(event) = parsed {
            assert_eq!(event.signature(), "5M6A9");
            assert!(!event.has_error());
            assert_eq!(event.logs().len(), 2);
        }
    }

    #[test]
    fn decode_dma_payload_rejects_invalid_utf8() {
        let decoded = decode_dma_payload(&[0x80, 0xFF, 0x00]);
        assert!(decoded.is_err());
    }

    #[test]
    fn unsupported_vendor_reports_error() {
        let adapter = FpgaFeedAdapter::new("unknown_vendor".to_owned(), false);
        let (sender, _receiver) = mpsc::unbounded_channel();

        let result = adapter.spawn_stream(sender);
        assert!(result.is_err());

        if let Err(error) = result {
            assert!(matches!(error, FpgaFeedError::UnsupportedVendor { .. }));
        }
    }

    #[test]
    fn direct_vendor_requires_device_path() {
        let adapter = FpgaFeedAdapter::new("exanic".to_owned(), false)
            .with_ingress_mode(FpgaIngressMode::DirectDevice)
            .with_direct_device_path("/tmp/slotstrike-missing-fpga-device".to_owned());

        let ready = adapter.validate_ready();
        assert!(ready.is_err());

        if let Err(error) = ready {
            assert!(matches!(
                error,
                FpgaFeedError::DirectDevicePathMissing { .. }
            ));
        }
    }

    #[test]
    fn external_vendor_requires_socket_path() {
        let adapter = FpgaFeedAdapter::new("exanic".to_owned(), false)
            .with_ingress_mode(FpgaIngressMode::ExternalSocket)
            .with_dma_socket_path("/tmp/slotstrike-missing-fpga.sock".to_owned());

        let ready = adapter.validate_ready();
        assert!(ready.is_err());

        if let Err(error) = ready {
            assert!(matches!(error, FpgaFeedError::DmaSocketPathMissing { .. }));
        }
    }

    #[tokio::test]
    async fn mock_dma_vendor_streams_events() {
        let adapter = FpgaFeedAdapter::with_inline_dma_payload(
            MOCK_DMA_VENDOR.to_owned(),
            false,
            format!(
                "signature=abc123\nhas_error=false\nlog=Program {} logs\nlog=Program log: initialize2",
                RAYDIUM_V4_PROGRAM_ID
            )
            .as_bytes(),
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();

        let spawned = adapter.spawn_stream(sender);
        assert!(spawned.is_ok());

        let received = tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await;
        assert!(received.is_ok());

        if let Ok(event) = received {
            assert!(event.is_some());

            if let Some(event) = event {
                assert_eq!(event.signature, "abc123");
                assert_eq!(event.logs.len(), 2);
                assert!(!event.has_error);
                assert_eq!(event.ingress.source.as_str(), "fpga_dma");
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_socket_vendor_streams_events() {
        let socket_path = unique_socket_path("slotstrike-fpga-test");
        let _remove_before = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path);
        assert!(listener.is_ok());
        let listener = if let Ok(listener) = listener {
            listener
        } else {
            return;
        };

        let frame_payload = format!(
            "signature=external123\nhas_error=0\nlog=Program {}\nlog=Program log: initialize2",
            RAYDIUM_V4_PROGRAM_ID
        );
        let encoded_payload = BASE64_STANDARD.encode(frame_payload.as_bytes());
        let frame_json = format!(
            "{{\"hardware_timestamp_ns\":123456789,\"payload_base64\":\"{}\"}}\n",
            encoded_payload
        );

        let thread_frame_json = frame_json.clone();
        let server_thread = thread::spawn(move || {
            for accept_attempt in 0..2 {
                let accepted = listener.accept();
                if let Ok((mut stream, _address)) = accepted
                    && accept_attempt == 1
                {
                    let _write_result = stream.write_all(thread_frame_json.as_bytes());
                    let _flush_result = stream.flush();
                }
            }
        });

        let adapter = FpgaFeedAdapter::new("xilinx".to_owned(), false)
            .with_ingress_mode(FpgaIngressMode::ExternalSocket)
            .with_dma_socket_path(socket_path.to_string_lossy().to_string());
        let (sender, mut receiver) = mpsc::unbounded_channel();

        let spawned = adapter.spawn_stream(sender);
        assert!(spawned.is_ok());

        let received = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await;
        assert!(received.is_ok());

        if let Ok(event) = received {
            assert!(event.is_some());

            if let Some(event) = event {
                assert_eq!(event.signature, "external123");
                assert!(!event.has_error);
                assert_eq!(event.ingress.source.as_str(), "fpga_dma");
                assert_eq!(event.ingress.hardware_timestamp_ns, Some(123456789));
            }
        }

        let join_result = server_thread.join();
        assert!(join_result.is_ok());

        let _remove_after = std::fs::remove_file(&socket_path);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_device_vendor_streams_events() {
        let device_path = unique_device_file_path("slotstrike-fpga-direct-device-test");
        let payload = format!(
            "signature=direct123\nhas_error=0\nlog=Program {}\nlog=Program log: initialize2",
            RAYDIUM_V4_PROGRAM_ID
        );
        let encoded_payload = BASE64_STANDARD.encode(payload.as_bytes());
        let frame_line = format!("{}\n", encoded_payload);
        let write_result = std::fs::write(&device_path, frame_line.as_bytes());
        assert!(write_result.is_ok());

        let adapter = FpgaFeedAdapter::new("xilinx".to_owned(), false)
            .with_ingress_mode(FpgaIngressMode::DirectDevice)
            .with_direct_device_path(device_path.to_string_lossy().to_string());
        let (sender, mut receiver) = mpsc::unbounded_channel();

        let spawned = adapter.spawn_stream(sender);
        assert!(spawned.is_ok());

        let received = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await;
        assert!(received.is_ok());

        if let Ok(event) = received {
            assert!(event.is_some());

            if let Some(event) = event {
                assert_eq!(event.signature, "direct123");
                assert!(!event.has_error);
                assert_eq!(event.ingress.source.as_str(), "fpga_dma");
            }
        }

        let _remove_after = std::fs::remove_file(&device_path);
    }

    #[test]
    fn deterministic_prefilter_accepts_pool_creation() {
        let payload = format!(
            "log=Program {}\nlog=Program log: initialize2",
            RAYDIUM_V4_PROGRAM_ID
        );
        assert!(is_pool_creation_dma_payload(payload.as_bytes()));
    }

    #[test]
    fn deterministic_prefilter_rejects_swap_frame() {
        let payload = format!(
            "log=Program {}\nlog=Program log: SwapBaseIn",
            RAYDIUM_STANDARD_AMM_PROGRAM_ID
        );
        assert!(!is_pool_creation_dma_payload(payload.as_bytes()));
    }

    #[cfg(unix)]
    fn unique_socket_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map_or(0, |value| value.as_nanos());
        let file_name = format!("{}-{}-{}.sock", prefix, std::process::id(), nanos);
        PathBuf::from("/tmp").join(file_name)
    }

    #[cfg(unix)]
    fn unique_device_file_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map_or(0, |value| value.as_nanos());
        let file_name = format!("{}-{}-{}.txt", prefix, std::process::id(), nanos);
        PathBuf::from("/tmp").join(file_name)
    }
}
