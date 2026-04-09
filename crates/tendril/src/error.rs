use mcp_cli::{ErrorCategory, McpCliError, StructuredError};
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

    #[error("MCP/JSON error: {message}")]
    Mcp { message: String },
}

impl TendrilError {
    #[must_use]
    pub fn not_implemented(command: &'static str) -> Self {
        Self::NotImplemented { command }
    }

    #[must_use]
    pub fn mcp(error: &McpCliError) -> Self {
        Self::Mcp {
            message: error.to_string(),
        }
    }

    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::NotImplemented { .. } => ErrorCategory::UnsupportedCapability,
            Self::Config { .. } => ErrorCategory::ConfigError,
            Self::Adapter(error) => error.category(),
            Self::Mcp { .. } => ErrorCategory::SerializationError,
        }
    }
}

impl StructuredError for TendrilError {
    fn category(&self) -> ErrorCategory {
        self.category()
    }

    fn message(&self) -> String {
        self.to_string()
    }
}
