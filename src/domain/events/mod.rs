use std::time::{SystemTime, UNIX_EPOCH};

const HARDWARE_TIMESTAMP_MAX_SKEW_NS: u64 = 5_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressSource {
    FpgaDma,
    KernelBypass,
    StandardTcp,
}

impl IngressSource {
    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FpgaDma => "fpga_dma",
            Self::KernelBypass => "kernel_bypass",
            Self::StandardTcp => "standard_tcp",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngressMetadata {
    pub source: IngressSource,
    pub hardware_timestamp_ns: Option<u64>,
    pub received_timestamp_ns: u64,
    pub normalized_timestamp_ns: u64,
}

impl IngressMetadata {
    #[inline(always)]
    pub const fn from_hardware_clock(
        source: IngressSource,
        hardware_timestamp_ns: Option<u64>,
        received_timestamp_ns: u64,
    ) -> Self {
        Self {
            source,
            hardware_timestamp_ns,
            received_timestamp_ns,
            normalized_timestamp_ns: normalize_hardware_timestamp_ns(
                hardware_timestamp_ns,
                received_timestamp_ns,
            ),
        }
    }

    #[inline(always)]
    pub const fn from_receive_clock(source: IngressSource, received_timestamp_ns: u64) -> Self {
        Self::from_hardware_clock(source, None, received_timestamp_ns)
    }
}

#[derive(Clone, Debug)]
pub struct RawLogEvent {
    pub signature: String,
    pub logs: Vec<String>,
    pub has_error: bool,
    pub ingress: IngressMetadata,
}

#[inline(always)]
pub const fn normalize_hardware_timestamp_ns(
    hardware_timestamp_ns: Option<u64>,
    received_timestamp_ns: u64,
) -> u64 {
    match hardware_timestamp_ns {
        Some(value) if value != 0 => {
            let min = received_timestamp_ns.saturating_sub(HARDWARE_TIMESTAMP_MAX_SKEW_NS);
            let max = received_timestamp_ns.saturating_add(HARDWARE_TIMESTAMP_MAX_SKEW_NS);
            if value < min || value > max {
                received_timestamp_ns
            } else {
                value
            }
        }
        _ => received_timestamp_ns,
    }
}

#[inline(always)]
pub fn unix_timestamp_now_ns() -> u64 {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0;
    };

    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{
        IngressMetadata, IngressSource, normalize_hardware_timestamp_ns, unix_timestamp_now_ns,
    };

    #[test]
    fn accepts_hardware_timestamp_within_skew_window() {
        let receive_ns = 10_000_000_000_u64;
        let hardware_ns = receive_ns.saturating_sub(250_000);

        assert_eq!(
            normalize_hardware_timestamp_ns(Some(hardware_ns), receive_ns),
            hardware_ns
        );
    }

    #[test]
    fn clamps_outlier_hardware_timestamp_to_receive_clock() {
        let receive_ns = 10_000_000_000_u64;
        let outlier_ns = receive_ns.saturating_add(10_000_000_000);

        assert_eq!(
            normalize_hardware_timestamp_ns(Some(outlier_ns), receive_ns),
            receive_ns
        );
    }

    #[test]
    fn builds_receive_clock_metadata() {
        let receive_ns = unix_timestamp_now_ns();
        let metadata = IngressMetadata::from_receive_clock(IngressSource::StandardTcp, receive_ns);

        assert_eq!(metadata.source, IngressSource::StandardTcp);
        assert_eq!(metadata.hardware_timestamp_ns, None);
        assert_eq!(metadata.normalized_timestamp_ns, receive_ns);
    }
}
