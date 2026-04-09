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

        Ok(())
    }
}

impl Default for CaptureDefaults {
    fn default() -> Self {
        Self {
            format: ImageFormat::default(),
            compression: 85,
            max_width: None,
            max_height: None,
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{ConfigPaths, ImageFormat, LogLevel, TendrilConfig, detect_paths};

    #[test]
    fn defaults_cover_initial_capture_preferences() {
        let config = TendrilConfig::default();

        assert_eq!(config.capture.format, ImageFormat::Png);
        assert_eq!(config.capture.compression, 85);
        assert_eq!(config.logging.level, LogLevel::Info);
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
        let path = std::env::temp_dir().join(unique_name("missing-config"));

        let config = TendrilConfig::load_from_file(&path).expect("missing config uses defaults");

        assert_eq!(config, TendrilConfig::default());
    }

    #[test]
    fn load_from_yaml_applies_partial_defaults() {
        let path = write_temp_config(
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
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn invalid_yaml_values_are_rejected() {
        let path = write_temp_config(
            r"
capture:
  compression: 255
",
        );

        let error = TendrilConfig::load_from_file(&path).expect_err("invalid config should fail");

        assert_eq!(error.code(), "invalid_config");
        std::fs::remove_file(path).ok();
    }

    fn write_temp_config(contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(unique_name("tendril-config"));
        std::fs::write(&path, contents).expect("temp config should be writable");
        path
    }

    fn unique_name(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        format!("{prefix}-{nanos}.yaml")
    }
}
