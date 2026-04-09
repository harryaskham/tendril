use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable categories for structured JSON and MCP errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    UnsupportedCapability,
    MissingPermission,
    TargetNotFound,
    PlatformAdapterFailure,
    ExecutionFailure,
    ConfigError,
    SerializationError,
}

/// Structured error payload shared by CLI and MCP surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonError {
    pub category: ErrorCategory,
    pub message: String,
}

impl JsonError {
    #[must_use]
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

/// Structured success/error envelope for machine-readable command responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum JsonEnvelope<T> {
    Success { data: T },
    Error { error: JsonError },
}

impl<T> JsonEnvelope<T> {
    #[must_use]
    pub fn success(data: T) -> Self {
        Self::Success { data }
    }

    #[must_use]
    pub fn error(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self::Error {
            error: JsonError::new(category, message),
        }
    }
}

/// Minimal stdio server metadata for future MCP wiring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioServerConfig {
    pub server_name: String,
    pub server_version: String,
}

/// Errors surfaced by the reusable MCP façade.
#[derive(Debug, Error)]
pub enum McpCliError {
    #[error("MCP stdio scaffolding is not implemented yet")]
    Unimplemented,
}

/// Placeholder entry point for future MCP stdio wiring.
pub fn serve_stdio(_config: &StdioServerConfig) -> Result<(), McpCliError> {
    Err(McpCliError::Unimplemented)
}

#[cfg(test)]
mod tests {
    use super::{ErrorCategory, JsonEnvelope};
    use serde_json::json;

    #[test]
    fn success_envelope_serializes_with_status_tag() {
        let envelope = JsonEnvelope::success(json!({ "crate": "mcp-cli" }));

        let value = serde_json::to_value(envelope).expect("success envelope serializes");

        assert_eq!(value["status"], "success");
        assert_eq!(value["data"]["crate"], "mcp-cli");
    }

    #[test]
    fn error_envelope_serializes_with_structured_category() {
        let envelope: JsonEnvelope<()> =
            JsonEnvelope::error(ErrorCategory::Validation, "placeholder validation failure");

        let value = serde_json::to_value(envelope).expect("error envelope serializes");

        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["category"], "validation");
    }
}
