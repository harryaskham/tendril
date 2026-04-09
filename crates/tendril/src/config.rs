use std::env;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const CONFIG_DIR_ENV: &str = "TENDRIL_CONFIG_DIR";

/// Runtime configuration scaffold for machine-local defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TendrilConfig {
    pub capture: CaptureDefaults,
}

/// Screenshot defaults stored in `$TENDRIL_CONFIG_DIR/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureDefaults {
    pub format: ImageFormat,
    pub compression: u8,
}

impl Default for CaptureDefaults {
    fn default() -> Self {
        Self {
            format: ImageFormat::default(),
            compression: 85,
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
    use super::{ConfigPaths, ImageFormat, TendrilConfig, detect_paths};

    #[test]
    fn defaults_cover_initial_capture_preferences() {
        let config = TendrilConfig::default();

        assert_eq!(config.capture.format, ImageFormat::Png);
        assert_eq!(config.capture.compression, 85);
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
}
