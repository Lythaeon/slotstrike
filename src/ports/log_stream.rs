use std::{
    error::Error,
    fmt::{Display, Formatter},
};

use tokio::sync::mpsc;

use crate::domain::events::RawLogEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LogStreamError {
    Unavailable(String),
}

impl Display for LogStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "{}", message),
        }
    }
}

impl Error for LogStreamError {}

pub trait LogStreamPort: Send + Sync {
    fn path_name(&self) -> &'static str;
    fn spawn_stream(
        &self,
        sender: mpsc::UnboundedSender<RawLogEvent>,
    ) -> Result<(), LogStreamError>;
}
