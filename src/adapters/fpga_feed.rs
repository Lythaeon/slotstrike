use std::{collections::VecDeque, sync::Arc, thread};

use tokio::sync::mpsc;

use crate::{
    domain::events::{
        IngressMetadata, IngressSource, RawLogEvent, normalize_hardware_timestamp_ns,
        unix_timestamp_now_ns,
    },
    ports::fpga_feed::{FpgaFeedError, FpgaFeedPort},
    slices::sniper::pool_filter::is_pool_creation_dma_payload,
};

const MOCK_DMA_VENDOR: &str = "mock_dma";
const MOCK_DMA_FRAME_ENV: &str = "FPGA_DMA_MOCK_FRAME";

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
    mock_dma_payload: Option<Arc<[u8]>>,
}

impl FpgaFeedAdapter {
    #[expect(
        clippy::missing_const_for_fn,
        reason = "runtime constructor takes owned String and is not used in const contexts"
    )]
    pub fn new(vendor: String, verbose: bool) -> Self {
        Self {
            vendor,
            verbose,
            mock_dma_payload: None,
        }
    }

    fn bootstrap_dma_ring(&self) -> Result<DmaRing, FpgaFeedError> {
        if !self.vendor.eq_ignore_ascii_case(MOCK_DMA_VENDOR) {
            return Err(FpgaFeedError::Unavailable(format!(
                "vendor '{}' does not expose FPGA DMA ring integration yet",
                self.vendor
            )));
        }

        let payload = self.resolve_mock_dma_payload().ok_or_else(|| {
            FpgaFeedError::Unavailable(format!(
                "mock FPGA DMA ring requires '{}' environment payload",
                MOCK_DMA_FRAME_ENV
            ))
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

    pub fn with_mock_dma_payload(vendor: String, verbose: bool, payload: &[u8]) -> Self {
        Self {
            vendor,
            verbose,
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
                "FPGA feed enabled (vendor={}, verbose=true, hardware timestamps active, DMA ring path)",
                self.vendor
            )
        } else {
            format!("FPGA feed enabled (vendor={}, verbose=false)", self.vendor)
        }
    }

    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), FpgaFeedError> {
        let mut dma_ring = self.bootstrap_dma_ring()?;
        let verbose = self.verbose;

        thread::spawn(move || {
            while let Some(frame) = dma_ring.pop() {
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
                    continue;
                }

                let parsed = match FpgaFeedAdapter::decode_dma_frame(&frame) {
                    Ok(value) => value,
                    Err(error) => {
                        log::warn!("FPGA DMA decode failed: {}", error);
                        continue;
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
                    return;
                }
            }
        });

        Ok(())
    }
}

pub fn decode_dma_payload(payload: &[u8]) -> Result<DecodedDmaPayload, FpgaFeedError> {
    let payload = std::str::from_utf8(payload).map_err(|_parse_error| {
        FpgaFeedError::InvalidFrame("FPGA DMA payload is not valid UTF-8".to_owned())
    })?;

    let mut signature: Option<String> = None;
    let mut logs = Vec::new();
    let mut has_error = false;

    for line in payload.lines() {
        if let Some(value) = line.strip_prefix("signature=") {
            if value.trim().is_empty() {
                return Err(FpgaFeedError::InvalidFrame(
                    "FPGA DMA frame contains empty signature".to_owned(),
                ));
            }

            signature = Some(value.trim().to_owned());
            continue;
        }

        if let Some(value) = line.strip_prefix("has_error=") {
            has_error = parse_bool_flag(value).ok_or_else(|| {
                FpgaFeedError::InvalidFrame("FPGA DMA frame has invalid has_error flag".to_owned())
            })?;
            continue;
        }

        if let Some(value) = line.strip_prefix("log=") {
            logs.push(value.to_owned());
        }
    }

    let signature = signature.ok_or_else(|| {
        FpgaFeedError::InvalidFrame("FPGA DMA frame missing signature field".to_owned())
    })?;

    if logs.is_empty() {
        return Err(FpgaFeedError::InvalidFrame(
            "FPGA DMA frame does not contain logs".to_owned(),
        ));
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

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
    fn non_mock_vendor_reports_unavailable_dma_ring() {
        let adapter = FpgaFeedAdapter::new("exanic".to_owned(), false);
        let (sender, _receiver) = mpsc::unbounded_channel();

        let result = adapter.spawn_stream(sender);
        assert!(result.is_err());

        if let Err(error) = result {
            assert!(matches!(error, FpgaFeedError::Unavailable(_)));
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
}
