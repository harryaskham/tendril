use mcp_cli::ErrorCategory;
use thiserror::Error;

use crate::platform::PlatformAdapterError;

#[derive(Debug, Error)]
pub enum TendrilError {
    #[error("The `{command}` command is scaffolded but not implemented yet")]
    NotImplemented { command: &'static str },

    #[error("Configuration error: {message}")]
    Config { message: String },

    #[error(transparent)]
    Adapter(#[from] PlatformAdapterError),
}

impl TendrilError {
    #[must_use]
    pub fn not_implemented(command: &'static str) -> Self {
        Self::NotImplemented { command }
    }

    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::NotImplemented { .. } => ErrorCategory::UnsupportedCapability,
            Self::Config { .. } => ErrorCategory::ConfigError,
            Self::Adapter(error) => error.category(),
        }
    }
}
