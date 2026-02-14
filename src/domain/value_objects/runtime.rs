use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
    num::NonZeroUsize,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelBypassEngine {
    AfXdp,
    Dpdk,
    OpenOnload,
    AfXdpOrDpdkExternal,
}

impl KernelBypassEngine {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "af_xdp" => Some(Self::AfXdp),
            "dpdk" => Some(Self::Dpdk),
            "openonload" | "onload" => Some(Self::OpenOnload),
            "af_xdp_or_dpdk_external" => Some(Self::AfXdpOrDpdkExternal),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AfXdp => "af_xdp",
            Self::Dpdk => "dpdk",
            Self::OpenOnload => "openonload",
            Self::AfXdpOrDpdkExternal => "af_xdp_or_dpdk_external",
        }
    }
}

impl Display for KernelBypassEngine {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TxSubmissionMode {
    Jito,
    Direct,
}

impl TxSubmissionMode {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "jito" => Some(Self::Jito),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jito => "jito",
            Self::Direct => "direct",
        }
    }
}

impl Display for TxSubmissionMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PriorityFeesMicrolamports(u64);

impl PriorityFeesMicrolamports {
    #[inline(always)]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReplayEventCount(NonZeroUsize);

impl ReplayEventCount {
    pub fn new(value: usize) -> Result<Self, &'static str> {
        NonZeroUsize::new(value)
            .map(Self)
            .ok_or("replay event count must be greater than 0")
    }

    #[inline(always)]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ReplayBurstSize(NonZeroUsize);

impl ReplayBurstSize {
    pub fn new(value: usize) -> Result<Self, &'static str> {
        NonZeroUsize::new(value)
            .map(Self)
            .ok_or("replay burst size must be greater than 0")
    }

    #[inline(always)]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NonEmptyText(Arc<str>);

impl NonEmptyText {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("text value must not be empty");
        }

        Ok(Self(value))
    }

    #[inline(always)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for NonEmptyText {
    #[inline(always)]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for NonEmptyText {
    #[inline(always)]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Display for NonEmptyText {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for NonEmptyText {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(Arc::<str>::from(value))
    }
}

impl TryFrom<&str> for NonEmptyText {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(Arc::<str>::from(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KernelBypassEngine, NonEmptyText, PriorityFeesMicrolamports, ReplayBurstSize,
        ReplayEventCount, TxSubmissionMode,
    };

    #[test]
    fn parses_supported_kernel_bypass_engines() {
        assert_eq!(
            KernelBypassEngine::parse("af_xdp"),
            Some(KernelBypassEngine::AfXdp)
        );
        assert_eq!(
            KernelBypassEngine::parse("DPDK"),
            Some(KernelBypassEngine::Dpdk)
        );
        assert_eq!(
            KernelBypassEngine::parse("af_xdp_or_dpdk_external"),
            Some(KernelBypassEngine::AfXdpOrDpdkExternal)
        );
        assert_eq!(
            KernelBypassEngine::parse("openonload"),
            Some(KernelBypassEngine::OpenOnload)
        );
        assert_eq!(
            KernelBypassEngine::parse("ONLOAD"),
            Some(KernelBypassEngine::OpenOnload)
        );
    }

    #[test]
    fn rejects_invalid_kernel_bypass_engine() {
        assert_eq!(KernelBypassEngine::parse("invalid"), None);
    }

    #[test]
    fn parses_tx_submission_mode() {
        assert_eq!(
            TxSubmissionMode::parse("jito"),
            Some(TxSubmissionMode::Jito)
        );
        assert_eq!(
            TxSubmissionMode::parse("DIRECT"),
            Some(TxSubmissionMode::Direct)
        );
    }

    #[test]
    fn rejects_invalid_tx_submission_mode() {
        assert_eq!(TxSubmissionMode::parse("invalid"), None);
    }

    #[test]
    fn requires_non_empty_text() {
        assert!(NonEmptyText::try_from("vendor".to_owned()).is_ok());
        assert!(NonEmptyText::try_from(" ".to_owned()).is_err());
    }

    #[test]
    fn keeps_priority_fee_scalar() {
        let value = PriorityFeesMicrolamports::new(42);
        assert_eq!(value.as_u64(), 42);
    }

    #[test]
    fn enforces_non_zero_replay_counts() {
        assert!(ReplayEventCount::new(1).is_ok());
        assert!(ReplayEventCount::new(0).is_err());
        assert!(ReplayBurstSize::new(1).is_ok());
        assert!(ReplayBurstSize::new(0).is_err());
    }
}
