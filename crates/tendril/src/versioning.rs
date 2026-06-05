use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::cli::VersionBumpLevel;
use crate::error::TendrilError;

const VERSIONED_PACKAGE_NAMES: &[&str] = &["mcp-cli", "tendril", "tendril-win32"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionBumpOutput {
    pub previous_version: String,
    pub new_version: String,
    pub level: VersionBumpLevel,
    pub updated_files: Vec<String>,
    pub commit: String,
    pub tag: String,
}

pub fn execute_version_bump(level: VersionBumpLevel) -> Result<VersionBumpOutput, TendrilError> {
    let repo_root = discover_repo_root()?;
    ensure_clean_worktree(&repo_root)?;

    let previous_version = read_workspace_version(&repo_root.join("Cargo.toml"))?;
    let new_version = bump_version(&previous_version, level)?;
    let updated_files = update_version_files(&repo_root, &previous_version, &new_version)?;

    git_add(&repo_root, &updated_files)?;
    let commit_message = format!("chore(release): bump tendril to v{new_version}");
    let commit = git_commit(&repo_root, &commit_message)?;

    Ok(VersionBumpOutput {
        previous_version,
        new_version: new_version.clone(),
        level,
        updated_files,
        commit,
        tag: format!("v{new_version}"),
    })
}

#[must_use]
pub fn render_version_bump_human(output: &VersionBumpOutput) -> String {
    format!(
        "bumped Tendril from {} to {} ({:?})\ncommit: {}\ntag: v{}\nupdated files:\n{}\n",
        output.previous_version,
        output.new_version,
        output.level,
        output.commit,
        output.new_version,
        output
            .updated_files
            .iter()
            .map(|path| format!("  - {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn discover_repo_root() -> Result<PathBuf, TendrilError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "version_bump_git_unavailable",
                format!("failed to run git while locating the repository root: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(git_error(
            "version_bump_not_in_git_repo",
            "git rev-parse",
            &output,
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(PathBuf::from(stdout.trim()))
}

fn ensure_clean_worktree(repo_root: &Path) -> Result<(), TendrilError> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "version_bump_git_status_failed",
                format!("failed to inspect git worktree status: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(git_error(
            "version_bump_git_status_failed",
            "git status",
            &output,
        ));
    }
    let dirty = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(TendrilError::validation(
            "version bump requires a clean tracked worktree before it can create a release commit",
        )
        .with_code("version_bump_dirty_worktree")
        .with_detail_entry("git_status", serde_json::json!(dirty)))
    }
}

fn read_workspace_version(manifest_path: &Path) -> Result<String, TendrilError> {
    let text =
        fs::read_to_string(manifest_path).map_err(|error| io_error(manifest_path, &error))?;
    extract_version_in_section(&text, "workspace.package").ok_or_else(|| {
        TendrilError::config_path(
            manifest_path,
            "could not find `version = \"...\"` in [workspace.package]",
        )
        .with_code("version_bump_missing_workspace_version")
    })
}

fn update_version_files(
    repo_root: &Path,
    previous_version: &str,
    new_version: &str,
) -> Result<Vec<String>, TendrilError> {
    let mut updated = Vec::new();
    update_workspace_manifest_version(
        &repo_root.join("Cargo.toml"),
        previous_version,
        new_version,
        &mut updated,
    )?;
    update_package_manifest_version(
        &repo_root.join("crates/mcp-cli/Cargo.toml"),
        previous_version,
        new_version,
        &mut updated,
    )?;
    let lock_path = repo_root.join("Cargo.lock");
    if lock_path.exists() {
        update_cargo_lock_versions(&lock_path, previous_version, new_version, &mut updated)?;
    }
    Ok(updated)
}

fn update_workspace_manifest_version(
    path: &Path,
    previous_version: &str,
    new_version: &str,
    updated: &mut Vec<String>,
) -> Result<(), TendrilError> {
    update_version_line(
        path,
        |line, in_workspace_package| {
            in_workspace_package
                && version_line_value(line).is_some_and(|version| version == previous_version)
        },
        new_version,
        updated,
    )
}

fn update_package_manifest_version(
    path: &Path,
    previous_version: &str,
    new_version: &str,
    updated: &mut Vec<String>,
) -> Result<(), TendrilError> {
    update_version_line(
        path,
        |line, _| version_line_value(line).is_some_and(|version| version == previous_version),
        new_version,
        updated,
    )
}

fn update_version_line(
    path: &Path,
    mut should_update: impl FnMut(&str, bool) -> bool,
    new_version: &str,
    updated: &mut Vec<String>,
) -> Result<(), TendrilError> {
    let text = fs::read_to_string(path).map_err(|error| io_error(path, &error))?;
    let mut in_workspace_package = false;
    let mut changed = false;
    let mut rendered = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_workspace_package = trimmed == "[workspace.package]";
        }
        if !changed && should_update(line, in_workspace_package) {
            let prefix = line.split_once("version").map_or("", |(prefix, _)| prefix);
            writeln!(rendered, "{prefix}version = \"{new_version}\"")
                .expect("writing to a String cannot fail");
            changed = true;
        } else {
            rendered.push_str(line);
            rendered.push('\n');
        }
    }
    if !changed {
        return Err(TendrilError::config_path(
            path,
            "did not find the expected version line to update",
        )
        .with_code("version_bump_expected_version_not_found"));
    }
    fs::write(path, rendered).map_err(|error| io_error(path, &error))?;
    push_relative_path(path, updated);
    Ok(())
}

fn update_cargo_lock_versions(
    path: &Path,
    previous_version: &str,
    new_version: &str,
    updated: &mut Vec<String>,
) -> Result<(), TendrilError> {
    let text = fs::read_to_string(path).map_err(|error| io_error(path, &error))?;
    let mut current_package: Option<String> = None;
    let mut changed = 0usize;
    let mut rendered = String::new();

    for line in text.lines() {
        if line.trim() == "[[package]]" {
            current_package = None;
        } else if let Some(name) = line
            .trim()
            .strip_prefix("name = \"")
            .and_then(|rest| rest.strip_suffix('"'))
        {
            current_package = Some(name.to_owned());
        }

        if current_package
            .as_deref()
            .is_some_and(|name| VERSIONED_PACKAGE_NAMES.contains(&name))
            && version_line_value(line).is_some_and(|version| version == previous_version)
        {
            let prefix = line.split_once("version").map_or("", |(prefix, _)| prefix);
            writeln!(rendered, "{prefix}version = \"{new_version}\"")
                .expect("writing to a String cannot fail");
            changed += 1;
        } else {
            rendered.push_str(line);
            rendered.push('\n');
        }
    }

    if changed == 0 {
        return Err(TendrilError::config_path(
            path,
            "did not find any Tendril package versions to update in Cargo.lock",
        )
        .with_code("version_bump_lock_versions_not_found"));
    }
    fs::write(path, rendered).map_err(|error| io_error(path, &error))?;
    push_relative_path(path, updated);
    Ok(())
}

fn git_add(repo_root: &Path, files: &[String]) -> Result<(), TendrilError> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .arg("add")
        .args(files)
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "version_bump_git_add_failed",
                format!("failed to stage version bump files: {error}"),
                None,
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_error("version_bump_git_add_failed", "git add", &output))
    }
}

fn git_commit(repo_root: &Path, message: &str) -> Result<String, TendrilError> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["commit", "-m", message])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "version_bump_git_commit_failed",
                format!("failed to create version bump commit: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(git_error(
            "version_bump_git_commit_failed",
            "git commit",
            &output,
        ));
    }
    let rev_parse = Command::new("git")
        .current_dir(repo_root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "version_bump_git_rev_parse_failed",
                format!("failed to read version bump commit id: {error}"),
                None,
            )
        })?;
    if rev_parse.status.success() {
        Ok(String::from_utf8_lossy(&rev_parse.stdout).trim().to_owned())
    } else {
        Err(git_error(
            "version_bump_git_rev_parse_failed",
            "git rev-parse",
            &rev_parse,
        ))
    }
}

fn git_error(code: &'static str, command: &str, output: &std::process::Output) -> TendrilError {
    TendrilError::execution_failure(
        code,
        format!(
            "{command} failed: {}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        None,
    )
}

fn io_error(path: &Path, error: &std::io::Error) -> TendrilError {
    TendrilError::config_path(path, error.to_string()).with_code("version_bump_io_error")
}

fn push_relative_path(path: &Path, updated: &mut Vec<String>) {
    let relative = path
        .strip_prefix(std::env::current_dir().unwrap_or_default())
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    updated.push(relative);
}

fn extract_version_in_section(text: &str, section: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed == format!("[{section}]");
            continue;
        }
        if in_section && let Some(version) = version_line_value(line) {
            return Some(version.to_owned());
        }
    }
    None
}

fn version_line_value(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix("version = \"")
        .and_then(|rest| rest.split_once('"'))
        .map(|(version, _)| version)
}

fn bump_version(version: &str, level: VersionBumpLevel) -> Result<String, TendrilError> {
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(TendrilError::validation(format!(
            "workspace version `{version}` is not a simple major.minor.patch semantic version"
        ))
        .with_code("version_bump_invalid_semver"));
    }
    let mut major = parse_semver_component(parts[0], "major")?;
    let mut minor = parse_semver_component(parts[1], "minor")?;
    let mut patch = parse_semver_component(parts[2], "patch")?;
    match level {
        VersionBumpLevel::Patch => patch += 1,
        VersionBumpLevel::Minor => {
            minor += 1;
            patch = 0;
        }
        VersionBumpLevel::Major => {
            major += 1;
            minor = 0;
            patch = 0;
        }
    }
    Ok(format!("{major}.{minor}.{patch}"))
}

fn parse_semver_component(value: &str, name: &str) -> Result<u64, TendrilError> {
    value.parse::<u64>().map_err(|_| {
        TendrilError::validation(format!("{name} semver component `{value}` is not numeric"))
            .with_code("version_bump_invalid_semver")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        bump_version, extract_version_in_section, render_version_bump_human,
        update_cargo_lock_versions, update_package_manifest_version,
        update_workspace_manifest_version, version_line_value, VersionBumpOutput,
    };
    use crate::cli::VersionBumpLevel;

    #[test]
    fn bumps_semver_components() {
        assert_eq!(
            bump_version("1.2.3", VersionBumpLevel::Patch).expect("patch bump"),
            "1.2.4"
        );
        assert_eq!(
            bump_version("1.2.3", VersionBumpLevel::Minor).expect("minor bump"),
            "1.3.0"
        );
        assert_eq!(
            bump_version("1.2.3", VersionBumpLevel::Major).expect("major bump"),
            "2.0.0"
        );
    }

    #[test]
    fn extracts_workspace_package_version_only() {
        let text = r#"
[package]
version = "9.9.9"

[workspace.package]
version = "1.2.3"
"#;
        assert_eq!(
            extract_version_in_section(text, "workspace.package"),
            Some("1.2.3".to_owned())
        );
    }

    #[test]
    fn updates_manifest_and_lock_versions() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root_manifest = tempdir.path().join("Cargo.toml");
        let package_manifest = tempdir.path().join("crates/mcp-cli/Cargo.toml");
        let lock = tempdir.path().join("Cargo.lock");
        std::fs::create_dir_all(package_manifest.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &root_manifest,
            "[workspace.package]\nversion = \"0.0.1\"\nedition = \"2024\"\n",
        )
        .expect("root manifest");
        std::fs::write(
            &package_manifest,
            "[package]\nname = \"mcp-cli\"\nversion = \"0.0.1\"\n",
        )
        .expect("package manifest");
        std::fs::write(
            &lock,
            "[[package]]\nname = \"tendril\"\nversion = \"0.0.1\"\n\n[[package]]\nname = \"serde\"\nversion = \"0.0.1\"\n",
        )
        .expect("lock");
        let mut updated = Vec::new();

        update_workspace_manifest_version(&root_manifest, "0.0.1", "0.1.0", &mut updated)
            .expect("workspace update");
        update_package_manifest_version(&package_manifest, "0.0.1", "0.1.0", &mut updated)
            .expect("package update");
        update_cargo_lock_versions(&lock, "0.0.1", "0.1.0", &mut updated).expect("lock update");

        assert!(
            std::fs::read_to_string(root_manifest)
                .expect("read")
                .contains("0.1.0")
        );
        assert!(
            std::fs::read_to_string(package_manifest)
                .expect("read")
                .contains("0.1.0")
        );
        let lock_text = std::fs::read_to_string(lock).expect("read");
        assert!(lock_text.contains("name = \"tendril\"\nversion = \"0.1.0\""));
        assert!(lock_text.contains("name = \"serde\"\nversion = \"0.0.1\""));
    }

    #[test]
    fn workspace_manifest_update_is_gated_to_the_workspace_package_section() {
        // A version line that sits in [package] (not [workspace.package]) is not a
        // workspace-package version, so the workspace updater leaves it alone and
        // reports the not-found error.
        let tempdir = tempfile::tempdir().expect("tempdir");
        let manifest = tempdir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"tendril\"\nversion = \"1.0.0\"\n",
        )
        .expect("manifest");
        let mut updated = Vec::new();
        let error = update_workspace_manifest_version(&manifest, "1.0.0", "1.1.0", &mut updated)
            .expect_err("a non-workspace-package version line should not match");
        assert_eq!(error.code(), "version_bump_expected_version_not_found");
        assert!(updated.is_empty());
        // The file is untouched by the failed workspace update.
        assert!(
            std::fs::read_to_string(&manifest)
                .expect("read")
                .contains("version = \"1.0.0\"")
        );
        // The package-level updater is not section-gated, so it rewrites the same line.
        update_package_manifest_version(&manifest, "1.0.0", "1.1.0", &mut updated)
            .expect("package update succeeds");
        assert!(
            std::fs::read_to_string(&manifest)
                .expect("read")
                .contains("version = \"1.1.0\"")
        );
    }

    #[test]
    fn version_line_update_reports_not_found_for_mismatched_previous_version() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let manifest = tempdir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[workspace.package]\nversion = \"2.0.0\"\n",
        )
        .expect("manifest");
        let mut updated = Vec::new();
        // The expected previous version does not match the file contents.
        let error = update_workspace_manifest_version(&manifest, "1.9.0", "2.1.0", &mut updated)
            .expect_err("a mismatched previous version should not match");
        assert_eq!(error.code(), "version_bump_expected_version_not_found");
        assert!(updated.is_empty());
    }

    #[test]
    fn renders_version_bump_summary() {
        let output = VersionBumpOutput {
            previous_version: "0.1.0".to_owned(),
            new_version: "0.2.0".to_owned(),
            level: VersionBumpLevel::Minor,
            updated_files: vec!["Cargo.toml".to_owned(), "Cargo.lock".to_owned()],
            commit: "abc1234".to_owned(),
            tag: "v0.2.0".to_owned(),
        };
        let rendered = render_version_bump_human(&output);
        assert!(rendered.contains("bumped Tendril from 0.1.0 to 0.2.0 (Minor)"));
        assert!(rendered.contains("commit: abc1234"));
        assert!(rendered.contains("tag: v0.2.0"));
        assert!(rendered.contains("  - Cargo.toml"));
        assert!(rendered.contains("  - Cargo.lock"));
    }

    #[test]
    fn version_line_value_extracts_quoted_version() {
        assert_eq!(version_line_value("version = \"1.2.3\""), Some("1.2.3"));
        assert_eq!(version_line_value("  version = \"0.0.1\"  "), Some("0.0.1"));
        assert_eq!(version_line_value("edition = \"2024\""), None);
        assert_eq!(version_line_value("not a version line"), None);
    }

    #[test]
    fn bump_version_rejects_malformed_semver() {
        let non_three = bump_version("1.2", VersionBumpLevel::Patch)
            .expect_err("two-part version should be rejected");
        assert_eq!(non_three.code(), "version_bump_invalid_semver");

        let non_numeric = bump_version("1.x.3", VersionBumpLevel::Patch)
            .expect_err("non-numeric component should be rejected");
        assert_eq!(non_numeric.code(), "version_bump_invalid_semver");
    }
}
