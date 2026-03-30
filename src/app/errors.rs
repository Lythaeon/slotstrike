use std::path::PathBuf;

use solana_client::client_error::ClientError;
use thiserror::Error;

use crate::{
    app::{logging::LoggingError, systemd::SystemdError},
    domain::settings::SettingsError,
};

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    ServiceCommand(#[from] SystemdError),
    #[error(transparent)]
    Logging(#[from] LoggingError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Keypair(#[from] KeypairLoadError),
    #[error(transparent)]
    Rulebook(#[from] RulebookLoadError),
    #[error(transparent)]
    WalletBalance(#[from] WalletBalanceError),
    #[error(transparent)]
    IngressStartup(#[from] IngressStartupError),
}

#[derive(Debug, Error)]
pub enum KeypairLoadError {
    #[error("failed to open keypair file at {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read keypair file at {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse keypair json at {path}")]
    ParseJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid keypair bytes at {path}")]
    InvalidBytes {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

#[derive(Debug, Error)]
pub enum RulebookLoadError {
    #[error("failed to read rules at startup")]
    Read {
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum WalletBalanceError {
    #[error("failed to read wallet balance")]
    Read {
        #[source]
        source: ClientError,
    },
}

#[derive(Debug, Error)]
pub enum IngressStartupError {
    #[error("failed to start SOF runtime: {detail}")]
    Sof { detail: String },
}
