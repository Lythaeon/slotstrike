use std::{
    error::Error,
    fmt::{Display, Formatter},
};

use tokio::sync::mpsc;

use crate::domain::events::RawLogEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FpgaFeedError {
    Unavailable(String),
    InvalidFrame(String),
}

impl Display for FpgaFeedError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "{}", message),
            Self::InvalidFrame(message) => write!(formatter, "{}", message),
        }
    }
}

impl Error for FpgaFeedError {}

pub trait FpgaFeedPort: Send + Sync {
    fn vendor(&self) -> &str;
    fn verbose(&self) -> bool;
    fn describe(&self) -> String;
    fn spawn_stream(&self, sender: mpsc::UnboundedSender<RawLogEvent>)
    -> Result<(), FpgaFeedError>;
}
