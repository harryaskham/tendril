use std::path::Path;

use mcp_cli::{ErrorCategory, JsonError, StructuredError};
use serde_json::{Value, json};
use thiserror::Error;

use crate::platform::PlatformAdapterError;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum TendrilError {
    #[error("validation error: {message}")]
    Validation {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("unsupported capability: {message}")]
    UnsupportedCapability {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("missing permission: {message}")]
    MissingPermission {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("target not found: {message}")]
    TargetNotFound {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("platform adapter failure: {message}")]
    PlatformAdapterFailure {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("execution failure: {message}")]
    ExecutionFailure {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("configuration error: {message}")]
    Config {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },

    #[error("serialization error: {message}")]
    Serialization {
        code: &'static str,
        message: String,
        details: Option<ErrorDetails>,
    },
}

#[derive(Debug, Error, Clone, PartialEq)]
#[error("structured tendril error details")]
pub struct ErrorDetails(pub Value);

impl TendrilError {
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            code: "invalid_input",
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn unsupported_capability(
        code: &'static str,
        message: impl Into<String>,
        details: Option<Value>,
    ) -> Self {
        Self::UnsupportedCapability {
            code,
            message: message.into(),
            details: details.map(ErrorDetails),
        }
    }

    #[must_use]
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            code: "config_error",
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn config_path(path: &Path, message: impl Into<String>) -> Self {
        Self::Config {
            code: "config_error",
            message: message.into(),
            details: Some(ErrorDetails(json!({ "path": path }))),
        }
    }

    #[must_use]
    pub fn serialization(message: impl Into<String>) -> Self {
        Self::Serialization {
            code: "serialization_error",
            message: message.into(),
            details: None,
        }
    }

    #[must_use]
    pub fn target_not_found(kind: &str, id: impl Into<String>) -> Self {
        let id = id.into();
        Self::TargetNotFound {
            code: "target_not_found",
            message: format!("{kind} `{id}` was not found"),
            details: Some(ErrorDetails(
                json!({ "target_kind": kind, "target_id": id }),
            )),
        }
    }

    #[must_use]
    pub fn execution_failure(
        code: &'static str,
        message: impl Into<String>,
        action_index: Option<usize>,
    ) -> Self {
        Self::ExecutionFailure {
            code,
            message: message.into(),
            details: action_index.map(|index| ErrorDetails(json!({ "action_index": index }))),
        }
    }

    #[must_use]
    pub fn not_implemented(command: &'static str) -> Self {
        Self::unsupported_capability(
            "command_not_implemented",
            format!("the `{command}` command is scaffolded but not implemented yet"),
            Some(json!({ "command": command })),
        )
    }

    #[must_use]
    pub fn with_field(self, field: impl Into<String>) -> Self {
        self.with_detail_entry("field", Value::String(field.into()))
    }

    #[must_use]
    pub fn with_detail_entry(self, key: &str, value: Value) -> Self {
        let key = key.to_owned();
        match self {
            Self::Validation {
                code,
                message,
                details,
            } => Self::Validation {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::UnsupportedCapability {
                code,
                message,
                details,
            } => Self::UnsupportedCapability {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::MissingPermission {
                code,
                message,
                details,
            } => Self::MissingPermission {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::TargetNotFound {
                code,
                message,
                details,
            } => Self::TargetNotFound {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::PlatformAdapterFailure {
                code,
                message,
                details,
            } => Self::PlatformAdapterFailure {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::ExecutionFailure {
                code,
                message,
                details,
            } => Self::ExecutionFailure {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::Config {
                code,
                message,
                details,
            } => Self::Config {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
            Self::Serialization {
                code,
                message,
                details,
            } => Self::Serialization {
                code,
                message,
                details: Some(merge_details(details, key, value)),
            },
        }
    }

    #[must_use]
    pub fn with_code(self, code: &'static str) -> Self {
        match self {
            Self::Validation {
                message, details, ..
            } => Self::Validation {
                code,
                message,
                details,
            },
            Self::UnsupportedCapability {
                message, details, ..
            } => Self::UnsupportedCapability {
                code,
                message,
                details,
            },
            Self::MissingPermission {
                message, details, ..
            } => Self::MissingPermission {
                code,
                message,
                details,
            },
            Self::TargetNotFound {
                message, details, ..
            } => Self::TargetNotFound {
                code,
                message,
                details,
            },
            Self::PlatformAdapterFailure {
                message, details, ..
            } => Self::PlatformAdapterFailure {
                code,
                message,
                details,
            },
            Self::ExecutionFailure {
                message, details, ..
            } => Self::ExecutionFailure {
                code,
                message,
                details,
            },
            Self::Config {
                message, details, ..
            } => Self::Config {
                code,
                message,
                details,
            },
            Self::Serialization {
                message, details, ..
            } => Self::Serialization {
                code,
                message,
                details,
            },
        }
    }

    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Validation { .. } => ErrorCategory::Validation,
            Self::UnsupportedCapability { .. } => ErrorCategory::UnsupportedCapability,
            Self::MissingPermission { .. } => ErrorCategory::MissingPermission,
            Self::TargetNotFound { .. } => ErrorCategory::TargetNotFound,
            Self::PlatformAdapterFailure { .. } => ErrorCategory::PlatformAdapterFailure,
            Self::ExecutionFailure { .. } => ErrorCategory::ExecutionFailure,
            Self::Config { .. } => ErrorCategory::ConfigError,
            Self::Serialization { .. } => ErrorCategory::SerializationError,
        }
    }

    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation { code, .. }
            | Self::UnsupportedCapability { code, .. }
            | Self::MissingPermission { code, .. }
            | Self::TargetNotFound { code, .. }
            | Self::PlatformAdapterFailure { code, .. }
            | Self::ExecutionFailure { code, .. }
            | Self::Config { code, .. }
            | Self::Serialization { code, .. } => code,
        }
    }

    #[must_use]
    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::Validation { details, .. }
            | Self::UnsupportedCapability { details, .. }
            | Self::MissingPermission { details, .. }
            | Self::TargetNotFound { details, .. }
            | Self::PlatformAdapterFailure { details, .. }
            | Self::ExecutionFailure { details, .. }
            | Self::Config { details, .. }
            | Self::Serialization { details, .. } => details.as_ref().map(|details| &details.0),
        }
    }

    #[must_use]
    pub fn to_json_error(&self) -> JsonError {
        JsonError::from_error(self)
    }
}

impl From<PlatformAdapterError> for TendrilError {
    fn from(error: PlatformAdapterError) -> Self {
        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                Self::UnsupportedCapability {
                    code: "unsupported_capability",
                    message: capability.message,
                    details: Some(ErrorDetails(json!({
                        "capability": capability.capability,
                        "platform": capability.platform,
                        "reason": capability.reason,
                        "suggested_action": capability.suggested_action,
                    }))),
                }
            }
            PlatformAdapterError::MissingPermission {
                capability,
                permission,
                platform,
                message,
                suggested_action,
            } => Self::MissingPermission {
                code: "missing_permission",
                message,
                details: Some(ErrorDetails(json!({
                    "capability": capability,
                    "permission": permission,
                    "platform": platform,
                    "suggested_action": suggested_action,
                }))),
            },
            PlatformAdapterError::AdapterFailure {
                operation,
                platform,
                message,
            } => Self::PlatformAdapterFailure {
                code: "platform_adapter_failure",
                message,
                details: Some(ErrorDetails(json!({
                    "operation": operation,
                    "platform": platform,
                }))),
            },
        }
    }
}

impl StructuredError for TendrilError {
    fn category(&self) -> ErrorCategory {
        self.category()
    }

    fn code(&self) -> String {
        self.code().to_owned()
    }

    fn message(&self) -> String {
        self.to_string()
    }

    fn details(&self) -> Option<Value> {
        self.details().cloned()
    }
}

fn merge_details(existing: Option<ErrorDetails>, key: String, value: Value) -> ErrorDetails {
    let mut object = existing
        .map(|details| details.0)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    object.insert(key, value);
    ErrorDetails(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::TendrilError;

    #[test]
    fn validation_errors_preserve_category_and_field_details() {
        let error = TendrilError::validation("bad value")
            .with_code("invalid_capture_input")
            .with_field("compression");
        let json_error = error.to_json_error();

        assert_eq!(json_error.category, mcp_cli::ErrorCategory::Validation);
        assert_eq!(json_error.code, "invalid_capture_input");
        assert_eq!(json_error.details.expect("details")["field"], "compression");
    }

    #[test]
    fn not_implemented_is_an_unsupported_capability() {
        let error = TendrilError::not_implemented("list");
        let json_error = error.to_json_error();

        assert_eq!(
            json_error.category,
            mcp_cli::ErrorCategory::UnsupportedCapability
        );
        assert_eq!(json_error.code, "command_not_implemented");
        assert_eq!(json_error.details.expect("details")["command"], "list");
    }
}
