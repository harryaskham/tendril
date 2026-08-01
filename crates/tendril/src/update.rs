use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "windows"))]
use updatable_cli::{AssetStrategy, Updater, UpdaterConfig};

use crate::cli::{UpdateAction, UpdateCommand};
use crate::error::TendrilError;

pub(crate) const DEFAULT_REPOSITORY: &str = "harryaskham/tendril";

/// Shared `updatable-cli` configuration used by both `tendril update` and the
/// MCP `self_update_*` tools.
#[cfg(not(target_os = "windows"))]
#[must_use]
pub fn updater_config() -> UpdaterConfig {
    let mut config = UpdaterConfig::new("tendril", env!("CARGO_PKG_VERSION"), DEFAULT_REPOSITORY);
    config.asset_strategy = AssetStrategy::TendrilStyle;
    config
}

#[cfg(not(target_os = "windows"))]
fn configured_updater(command: &UpdateCommand) -> Updater {
    let mut config = updater_config();
    if let Some(repository) = &command.repository {
        config.repo_slug.clone_from(repository);
    }
    if let Some(install_dir) = &command.install_dir {
        config.install_dir = Some(install_dir.clone());
    }
    Updater::new(config)
}

/// Stable Tendril envelope around the shared updater's status/check/run
/// results. Tendril owns presentation only; release discovery, checksum
/// verification, extraction, staging, and promotion all remain in
/// `updatable-cli`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum UpdateOutput {
    Status {
        tool: String,
        current_version: String,
        install_dir: String,
        installed_path: String,
        installed_exists: bool,
        next_path: String,
        next_staged: bool,
    },
    Check {
        tag: String,
        version: String,
        html_url: Option<String>,
        assets: Vec<String>,
        newer_than_current: bool,
    },
    Run {
        current_version: String,
        latest_version: String,
        staged: bool,
        promoted: bool,
        next_path: String,
        installed_path: String,
        note: Option<String>,
    },
}

#[cfg(not(target_os = "windows"))]
pub fn execute_update(command: &UpdateCommand) -> Result<UpdateOutput, TendrilError> {
    let updater = configured_updater(command);
    match command.action {
        UpdateAction::Status => updater
            .current_status()
            .map(|status| UpdateOutput::Status {
                tool: status.tool,
                current_version: status.current_version,
                install_dir: status.install_dir,
                installed_path: status.installed_path,
                installed_exists: status.installed_exists,
                next_path: status.next_path,
                next_staged: status.next_staged,
            })
            .map_err(|error| update_error("status", &error)),
        UpdateAction::Check => updater
            .check_latest()
            .map(|latest| UpdateOutput::Check {
                tag: latest.tag,
                version: latest.version,
                html_url: latest.html_url,
                assets: latest.assets,
                newer_than_current: latest.newer_than_current,
            })
            .map_err(|error| update_error("check", &error)),
        UpdateAction::Run => updater
            .run_update()
            .map(|outcome| UpdateOutput::Run {
                current_version: outcome.current_version,
                latest_version: outcome.latest_version,
                staged: outcome.staged,
                promoted: outcome.promoted,
                next_path: outcome.next_path,
                installed_path: outcome.installed_path,
                note: outcome.note,
            })
            .map_err(|error| update_error("run", &error)),
    }
}

#[cfg(target_os = "windows")]
pub fn execute_update(_command: &UpdateCommand) -> Result<UpdateOutput, TendrilError> {
    Err(TendrilError::unsupported_capability(
        "update_unsupported_platform",
        "the shared updatable-cli updater currently supports Linux and macOS release assets; install the Windows binary from source or a separately published Windows package",
        Some(serde_json::json!({
            "platform": "windows",
            "repository": DEFAULT_REPOSITORY,
            "updater": "updatable-cli",
        })),
    ))
}

#[cfg(not(target_os = "windows"))]
fn update_error(action: &'static str, error: &anyhow::Error) -> TendrilError {
    TendrilError::execution_failure(
        "self_update_failed",
        format!("updatable-cli {action} failed: {error:#}"),
        None,
    )
    .with_detail_entry("action", serde_json::json!(action))
    .with_detail_entry("updater", serde_json::json!("updatable-cli"))
}

#[must_use]
pub fn render_update_human(output: &UpdateOutput) -> String {
    match output {
        UpdateOutput::Status {
            current_version,
            install_dir,
            installed_path,
            installed_exists,
            next_path,
            next_staged,
            ..
        } => format!(
            "tendril update status\ncurrent version: {current_version}\ninstall dir: {install_dir}\ninstalled: {installed_exists} ({installed_path})\nstaged next: {next_staged} ({next_path})\n"
        ),
        UpdateOutput::Check {
            tag,
            version,
            assets,
            newer_than_current,
            ..
        } => format!(
            "tendril latest release\ntag: {tag}\nversion: {version}\nnewer than current: {newer_than_current}\nassets: {}\n",
            assets.join(", ")
        ),
        UpdateOutput::Run {
            current_version,
            latest_version,
            staged,
            promoted,
            next_path,
            installed_path,
            note,
        } => format!(
            "tendril update\ncurrent version: {current_version}\nlatest version: {latest_version}\nstaged: {staged}\npromoted: {promoted}\nnext path: {next_path}\ninstalled path: {installed_path}{}\n",
            note.as_deref()
                .map(|note| format!("\nnote: {note}"))
                .unwrap_or_default()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_REPOSITORY, UpdateOutput, render_update_human};

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn updater_config_uses_shared_tendril_style_contract() {
        let config = super::updater_config();
        assert_eq!(config.tool_name, "tendril");
        assert_eq!(config.repo_slug, DEFAULT_REPOSITORY);
        assert!(matches!(
            config.asset_strategy,
            updatable_cli::AssetStrategy::TendrilStyle
        ));
    }

    #[test]
    fn renders_shared_update_run_outcome() {
        let output = UpdateOutput::Run {
            current_version: "0.0.3".to_owned(),
            latest_version: "0.0.4".to_owned(),
            staged: true,
            promoted: true,
            next_path: "/tmp/bin/tendril_next".to_owned(),
            installed_path: "/tmp/bin/tendril".to_owned(),
            note: None,
        };
        let rendered = render_update_human(&output);
        assert!(rendered.contains("latest version: 0.0.4"));
        assert!(rendered.contains("promoted: true"));
        assert!(rendered.contains("installed path: /tmp/bin/tendril"));
    }

    #[test]
    fn default_repository_is_canonical() {
        assert_eq!(DEFAULT_REPOSITORY, "harryaskham/tendril");
    }

    #[test]
    fn update_output_round_trips_json() {
        let output = UpdateOutput::Check {
            tag: "v0.0.4".to_owned(),
            version: "0.0.4".to_owned(),
            html_url: Some("https://github.com/harryaskham/tendril/releases/tag/v0.0.4".to_owned()),
            assets: vec!["tendril-0.0.4-aarch64-darwin.tar.gz".to_owned()],
            newer_than_current: true,
        };
        let encoded = serde_json::to_value(&output).expect("serialize update output");
        let decoded = serde_json::from_value(encoded).expect("deserialize update output");
        assert_eq!(output, decoded);
    }
}
