use std::{
    borrow::Borrow,
    fmt::{Display, Formatter},
    num::NonZeroUsize,
    sync::Arc,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofIngressSource {
    Websocket,
    Grpc,
    PrivateShred,
}

impl SofIngressSource {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "websocket" | "ws" => Some(Self::Websocket),
            "grpc" | "yellowstone_grpc" => Some(Self::Grpc),
            "private_shred" | "private-propagation" | "shred" => Some(Self::PrivateShred),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Websocket => "websocket",
            Self::Grpc => "grpc",
            Self::PrivateShred => "private_shred",
        }
    }
}

impl Display for SofIngressSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofCommitmentLevel {
    Processed,
    Confirmed,
    Finalized,
}

impl SofCommitmentLevel {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "processed" => Some(Self::Processed),
            "confirmed" => Some(Self::Confirmed),
            "finalized" => Some(Self::Finalized),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
        }
    }
}

impl Display for SofCommitmentLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofGossipRuntimeMode {
    Full,
    BootstrapOnly,
    ControlPlaneOnly,
}

impl SofGossipRuntimeMode {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "full" => Some(Self::Full),
            "bootstrap_only" | "bootstrap-only" => Some(Self::BootstrapOnly),
            "control_plane_only" | "control-plane-only" | "topology_only" | "topology-only" => {
                Some(Self::ControlPlaneOnly)
            }
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::BootstrapOnly => "bootstrap_only",
            Self::ControlPlaneOnly => "control_plane_only",
        }
    }
}

impl Display for SofGossipRuntimeMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofTxMode {
    Rpc,
    Jito,
    Direct,
    Hybrid,
    Custom,
}

impl SofTxMode {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "rpc" | "rpc_only" => Some(Self::Rpc),
            "jito" | "jito_only" => Some(Self::Jito),
            "direct" | "direct_only" => Some(Self::Direct),
            "hybrid" => Some(Self::Hybrid),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Jito => "jito",
            Self::Direct => "direct",
            Self::Hybrid => "hybrid",
            Self::Custom => "custom",
        }
    }
}

impl Display for SofTxMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofTxStrategy {
    OrderedFallback,
    AllAtOnce,
}

impl SofTxStrategy {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "ordered_fallback" | "ordered" => Some(Self::OrderedFallback),
            "all_at_once" | "burst" => Some(Self::AllAtOnce),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OrderedFallback => "ordered_fallback",
            Self::AllAtOnce => "all_at_once",
        }
    }
}

impl Display for SofTxStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofTxRoute {
    Rpc,
    Jito,
    Direct,
}

impl SofTxRoute {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "rpc" => Some(Self::Rpc),
            "jito" => Some(Self::Jito),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rpc => "rpc",
            Self::Jito => "jito",
            Self::Direct => "direct",
        }
    }
}

impl Display for SofTxRoute {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofTxReliability {
    LowLatency,
    Balanced,
    HighReliability,
}

impl SofTxReliability {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "low_latency" | "low" => Some(Self::LowLatency),
            "balanced" => Some(Self::Balanced),
            "high_reliability" | "high" => Some(Self::HighReliability),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LowLatency => "low_latency",
            Self::Balanced => "balanced",
            Self::HighReliability => "high_reliability",
        }
    }
}

impl Display for SofTxReliability {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SofTxJitoTransport {
    JsonRpc,
    Grpc,
}

impl SofTxJitoTransport {
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "json_rpc" | "json-rpc" | "rpc" => Some(Self::JsonRpc),
            "grpc" => Some(Self::Grpc),
            _ => None,
        }
    }

    #[inline(always)]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::JsonRpc => "json_rpc",
            Self::Grpc => "grpc",
        }
    }
}

impl Display for SofTxJitoTransport {
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
        NonEmptyText, PriorityFeesMicrolamports, ReplayBurstSize, ReplayEventCount,
        SofGossipRuntimeMode, TxSubmissionMode,
    };

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
    fn parses_gossip_runtime_modes() {
        assert_eq!(
            SofGossipRuntimeMode::parse("full"),
            Some(SofGossipRuntimeMode::Full)
        );
        assert_eq!(
            SofGossipRuntimeMode::parse("bootstrap-only"),
            Some(SofGossipRuntimeMode::BootstrapOnly)
        );
        assert_eq!(
            SofGossipRuntimeMode::parse("topology_only"),
            Some(SofGossipRuntimeMode::ControlPlaneOnly)
        );
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
