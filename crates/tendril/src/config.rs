use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::TendrilError;

pub const CONFIG_DIR_ENV: &str = "TENDRIL_CONFIG_DIR";

/// Runtime configuration scaffold for machine-local defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TendrilConfig {
    pub capture: CaptureDefaults,
    pub logging: LoggingDefaults,
    pub execution_lock: ExecutionLockDefaults,
    /// Optional breakage/feedback reporting strategy (feedback-cli). When unset,
    /// Tendril resolves a webhook from `TENDRIL_FEEDBACK_WEBHOOK_URL`, else
    /// `FEEDBACK_WEBHOOK_BASE_URL` + the tendril hook name (default
    /// `tendril-feedback`), else the generic `FEEDBACK_WEBHOOK_URL`, and is
    /// otherwise silent. Set this to route Tendril breakages to a caco feedback
    /// endpoint / beads, the local `caco` CLI, or a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<feedback_cli::FeedbackConfig>,
}

impl TendrilConfig {
    pub fn load() -> Result<Self, TendrilError> {
        let paths = ConfigPaths::detect();
        Self::load_from_file(&paths.file)
    }

    pub fn load_from_file(path: &Path) -> Result<Self, TendrilError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path).map_err(|error| {
            TendrilError::config_path(path, format!("failed to read config file: {error}"))
        })?;
        let config: Self = serde_yaml::from_str(&contents).map_err(|error| {
            TendrilError::config_path(path, format!("failed to parse yaml config: {error}"))
        })?;
        config.validate_with_path(path)?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), TendrilError> {
        self.validate_with_path(Path::new("config.yaml"))
    }

    fn validate_with_path(&self, path: &Path) -> Result<(), TendrilError> {
        self.capture.validate(path)?;
        self.execution_lock.validate(path)?;
        Ok(())
    }
}

/// Screenshot defaults stored in `$TENDRIL_CONFIG_DIR/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureDefaults {
    pub format: ImageFormat,
    pub compression: u8,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    /// Default per-call backend timeout in milliseconds. `None` lets the
    /// platform adapter choose its own default (currently 10 000 ms).
    pub timeout_ms: Option<u64>,
}

impl CaptureDefaults {
    fn validate(&self, path: &Path) -> Result<(), TendrilError> {
        if self.compression > 100 {
            return Err(TendrilError::config_path(
                path,
                "capture.compression must be between 0 and 100",
            )
            .with_code("invalid_config")
            .with_field("capture.compression"));
        }

        if self.max_width == Some(0) {
            return Err(TendrilError::config_path(
                path,
                "capture.max_width must be greater than zero",
            )
            .with_code("invalid_config")
            .with_field("capture.max_width"));
        }

        if self.max_height == Some(0) {
            return Err(TendrilError::config_path(
                path,
                "capture.max_height must be greater than zero",
            )
            .with_code("invalid_config")
            .with_field("capture.max_height"));
        }

        if self.timeout_ms == Some(0) {
            return Err(TendrilError::config_path(
                path,
                "capture.timeout_ms must be greater than zero",
            )
            .with_code("invalid_config")
            .with_field("capture.timeout_ms"));
        }

        Ok(())
    }
}

impl Default for CaptureDefaults {
    fn default() -> Self {
        Self {
            format: ImageFormat::default(),
            compression: 85,
            // Default to 512x512 bounding box (aspect-preserving) so agent
            // contexts are not blown up by full-resolution base64 PNGs. Agents
            // wanting more detail can either override --max-width/--max-height
            // or capture specific window/quadrant targets. See bd-702f90.
            max_width: Some(512),
            max_height: Some(512),
            timeout_ms: None,
        }
    }
}

/// Initial image format scaffold for future capture work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingDefaults {
    pub level: LogLevel,
}

/// Host-local execution lock defaults used by side-effecting desktop-control commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionLockDefaults {
    /// Enable the default host-local Tendril execution lock/queue for `run`.
    pub enabled: bool,
    /// Maximum time to wait in the local queue before returning a structured timeout.
    pub timeout_ms: u64,
    /// Age of a lock/ticket heartbeat before it is considered stale and reaped.
    pub stale_ms: u64,
    /// Optional override for the lock root. Defaults to a host-local temp path
    /// namespaced by user and desktop session.
    pub path: Option<PathBuf>,
}

impl Default for ExecutionLockDefaults {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_ms: crate::execution_lock::DEFAULT_LOCK_TIMEOUT_MS,
            stale_ms: crate::execution_lock::DEFAULT_LOCK_STALE_MS,
            path: None,
        }
    }
}

impl ExecutionLockDefaults {
    fn validate(&self, path: &Path) -> Result<(), TendrilError> {
        if self.timeout_ms == 0 {
            return Err(TendrilError::config_path(
                path,
                "execution_lock.timeout_ms must be greater than zero",
            )
            .with_code("invalid_config")
            .with_field("execution_lock.timeout_ms"));
        }

        if self.stale_ms == 0 {
            return Err(TendrilError::config_path(
                path,
                "execution_lock.stale_ms must be greater than zero",
            )
            .with_code("invalid_config")
            .with_field("execution_lock.stale_ms"));
        }

        Ok(())
    }
}

impl Default for LoggingDefaults {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Returns true when this level emits more than warnings (i.e. INFO/DEBUG/TRACE).
    #[must_use]
    pub fn is_more_verbose_than_warn(self) -> bool {
        matches!(self, Self::Info | Self::Debug | Self::Trace)
    }
}

/// Derived config locations resolved from the current environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub dir: PathBuf,
    pub file: PathBuf,
}

impl ConfigPaths {
    #[must_use]
    pub fn detect() -> Self {
        detect_paths(
            env::var_os(CONFIG_DIR_ENV).map(PathBuf::from),
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("HOME").map(PathBuf::from),
        )
    }
}

fn detect_paths(
    config_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> ConfigPaths {
    let dir = config_dir
        .or_else(|| xdg_config_home.map(|dir| dir.join("tendril")))
        .or_else(|| home_dir.map(|dir| dir.join(".config/tendril")))
        .unwrap_or_else(|| PathBuf::from(".config/tendril"));
    let file = dir.join("config.yaml");

    ConfigPaths { dir, file }
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_level_verbosity_classification() {
        use super::LogLevel;
        assert!(!LogLevel::Error.is_more_verbose_than_warn());
        assert!(!LogLevel::Warn.is_more_verbose_than_warn());
        assert!(LogLevel::Info.is_more_verbose_than_warn());
        assert!(LogLevel::Debug.is_more_verbose_than_warn());
        assert!(LogLevel::Trace.is_more_verbose_than_warn());
    }
    use super::{
        ConfigPaths, ExecutionLockDefaults, ImageFormat, LogLevel, TendrilConfig, detect_paths,
    };

    #[test]
    fn defaults_cover_initial_capture_preferences() {
        let config = TendrilConfig::default();

        assert_eq!(config.capture.format, ImageFormat::Png);
        assert_eq!(config.capture.compression, 85);
        assert_eq!(config.logging.level, LogLevel::Info);
        assert_eq!(config.execution_lock, ExecutionLockDefaults::default());
    }

    #[test]
    fn config_dir_env_overrides_default_location() {
        let paths = detect_paths(
            Some(std::path::PathBuf::from("/tmp/tendril-config")),
            None,
            None,
        );

        assert_eq!(paths.dir, std::path::PathBuf::from("/tmp/tendril-config"));
        assert_eq!(
            paths.file,
            std::path::PathBuf::from("/tmp/tendril-config/config.yaml")
        );
    }

    #[test]
    fn home_dir_fallback_matches_spec_location() {
        let paths = detect_paths(None, None, Some(std::path::PathBuf::from("/home/alice")));

        assert_eq!(
            paths,
            ConfigPaths {
                dir: std::path::PathBuf::from("/home/alice/.config/tendril"),
                file: std::path::PathBuf::from("/home/alice/.config/tendril/config.yaml"),
            }
        );
    }

    #[test]
    fn load_from_missing_file_returns_defaults() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = tempdir.path().join("missing-config.yaml");

        let config = TendrilConfig::load_from_file(&path).expect("missing config uses defaults");

        assert_eq!(config, TendrilConfig::default());
    }

    #[test]
    fn load_from_yaml_applies_partial_defaults() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
capture:
  format: jpeg
logging:
  level: debug
",
        );

        let config = TendrilConfig::load_from_file(&path).expect("yaml config should load");

        assert_eq!(config.capture.format, ImageFormat::Jpeg);
        assert_eq!(config.capture.compression, 85);
        assert_eq!(config.logging.level, LogLevel::Debug);
        assert!(config.execution_lock.enabled);
    }

    #[test]
    fn execution_lock_config_can_override_defaults() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
execution_lock:
  enabled: false
  timeout_ms: 1234
  stale_ms: 5678
  path: /tmp/custom-tendril-lock
",
        );

        let config = TendrilConfig::load_from_file(&path).expect("yaml config should load");

        assert!(!config.execution_lock.enabled);
        assert_eq!(config.execution_lock.timeout_ms, 1234);
        assert_eq!(config.execution_lock.stale_ms, 5678);
        assert_eq!(
            config.execution_lock.path,
            Some(std::path::PathBuf::from("/tmp/custom-tendril-lock"))
        );
    }

    #[test]
    fn invalid_execution_lock_values_are_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
execution_lock:
  timeout_ms: 0
",
        );

        let error = TendrilConfig::load_from_file(&path).expect_err("invalid config should fail");

        assert_eq!(error.code(), "invalid_config");
        assert_eq!(
            error.details().unwrap()["field"],
            "execution_lock.timeout_ms"
        );
    }

    #[test]
    fn invalid_yaml_values_are_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
capture:
  compression: 255
",
        );

        let error = TendrilConfig::load_from_file(&path).expect_err("invalid config should fail");

        assert_eq!(error.code(), "invalid_config");
        assert_eq!(
            error.details().expect("details")["field"],
            "capture.compression"
        );
    }

    #[test]
    fn zero_capture_max_width_is_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
capture:
  max_width: 0
",
        );

        let error =
            TendrilConfig::load_from_file(&path).expect_err("zero max_width should be rejected");
        assert_eq!(error.code(), "invalid_config");
        assert_eq!(
            error.details().expect("details")["field"],
            "capture.max_width"
        );
    }

    #[test]
    fn zero_capture_max_height_is_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
capture:
  max_height: 0
",
        );

        let error =
            TendrilConfig::load_from_file(&path).expect_err("zero max_height should be rejected");
        assert_eq!(error.code(), "invalid_config");
        assert_eq!(
            error.details().expect("details")["field"],
            "capture.max_height"
        );
    }

    #[test]
    fn zero_capture_timeout_ms_is_rejected() {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let path = write_temp_config(
            tempdir.path(),
            r"
capture:
  timeout_ms: 0
",
        );

        let error =
            TendrilConfig::load_from_file(&path).expect_err("zero timeout_ms should be rejected");
        assert_eq!(error.code(), "invalid_config");
        assert_eq!(
            error.details().expect("details")["field"],
            "capture.timeout_ms"
        );
    }

    fn write_temp_config(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("config.yaml");
        std::fs::write(&path, contents).expect("temp config should be writable");
        path
    }
}
