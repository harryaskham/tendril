use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use serde::{Deserialize, Serialize};
use serde_json::json;
use updatable_cli::{AssetStrategy, UpdaterConfig};

use crate::cli::UpdateCommand;
use crate::error::TendrilError;

pub(crate) const DEFAULT_REPOSITORY: &str = "harryaskham/tendril";

#[must_use]
pub fn updater_config() -> UpdaterConfig {
    let mut config = UpdaterConfig::new("tendril", env!("CARGO_PKG_VERSION"), DEFAULT_REPOSITORY);
    config.asset_strategy = AssetStrategy::TendrilStyle;
    config
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOutput {
    pub repository: String,
    pub version: String,
    pub tag: String,
    pub platform: String,
    pub archive_url: String,
    pub checksum_url: String,
    pub install_path: PathBuf,
    pub installed: bool,
    pub verified_version: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdatePlan {
    repository: String,
    version: String,
    tag: String,
    platform: String,
    archive_name: String,
    checksum_name: String,
    archive_url: String,
    checksum_url: String,
    install_path: PathBuf,
}

pub fn execute_update(command: &UpdateCommand) -> Result<UpdateOutput, TendrilError> {
    let repository = command
        .repository
        .clone()
        .unwrap_or_else(|| DEFAULT_REPOSITORY.to_owned());
    let platform = release_target_for(std::env::consts::OS, std::env::consts::ARCH)?;
    let version = command.release_version.clone().map_or_else(
        || query_latest_release_version(&repository),
        |version| Ok(normalize_version(&version)),
    )?;
    let install_dir = command
        .install_dir
        .clone()
        .map_or_else(default_install_dir, Ok)?;
    let plan = build_update_plan(&repository, &version, &platform, &install_dir);

    let mut output = UpdateOutput {
        repository: plan.repository.clone(),
        version: plan.version.clone(),
        tag: plan.tag.clone(),
        platform: plan.platform.clone(),
        archive_url: plan.archive_url.clone(),
        checksum_url: plan.checksum_url.clone(),
        install_path: plan.install_path.clone(),
        installed: false,
        verified_version: None,
        notes: Vec::new(),
    };

    if command.dry_run {
        output
            .notes
            .push("Dry run requested; no files were downloaded or installed.".to_owned());
        return Ok(output);
    }

    install_update(&plan, &mut output)?;
    Ok(output)
}

#[must_use]
pub fn render_update_human(output: &UpdateOutput) -> String {
    let status = if output.installed {
        "installed"
    } else {
        "planned"
    };
    let verified = output.verified_version.as_deref().unwrap_or("not run");
    let notes = if output.notes.is_empty() {
        String::new()
    } else {
        format!(
            "notes:\n{}\n",
            output
                .notes
                .iter()
                .map(|note| format!("  - {note}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    format!(
        "{status} Tendril {tag} for {platform}\ninstall path: {path}\nverified version: {verified}\narchive: {archive}\nchecksum: {checksum}\n{notes}",
        tag = output.tag,
        platform = output.platform,
        path = output.install_path.display(),
        archive = output.archive_url,
        checksum = output.checksum_url,
    )
}

fn install_update(plan: &UpdatePlan, output: &mut UpdateOutput) -> Result<(), TendrilError> {
    let temp_root = std::env::temp_dir().join(format!("tendril-update-{}", std::process::id()));
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root).map_err(|error| io_error(&temp_root, &error))?;
    }
    fs::create_dir_all(&temp_root).map_err(|error| io_error(&temp_root, &error))?;
    let archive_path = temp_root.join(&plan.archive_name);
    let checksum_path = temp_root.join(&plan.checksum_name);
    let extract_dir = temp_root.join("extract");
    fs::create_dir_all(&extract_dir).map_err(|error| io_error(&extract_dir, &error))?;

    download_asset_to_path(
        &plan.repository,
        &plan.tag,
        &plan.archive_name,
        &plan.archive_url,
        &archive_path,
    )?;
    download_asset_to_path(
        &plan.repository,
        &plan.tag,
        &plan.checksum_name,
        &plan.checksum_url,
        &checksum_path,
    )?;
    verify_checksum(&archive_path, &checksum_path)?;
    extract_archive(&archive_path, &extract_dir)?;

    let extracted_binary = extract_dir
        .join(format!("tendril-{}-{}", plan.version, plan.platform))
        .join(release_binary_name(&plan.platform));
    if !extracted_binary.is_file() {
        return Err(TendrilError::target_not_found(
            "release binary",
            extracted_binary.display().to_string(),
        ));
    }

    let install_dir = plan.install_path.parent().ok_or_else(|| {
        TendrilError::validation("install path does not have a parent directory")
            .with_code("update_invalid_install_path")
    })?;
    fs::create_dir_all(install_dir).map_err(|error| io_error(install_dir, &error))?;
    fs::copy(&extracted_binary, &plan.install_path)
        .map_err(|error| io_error(&plan.install_path, &error))?;
    make_executable(&plan.install_path)?;

    let verified = verify_installed_version(&plan.install_path, &plan.version)?;
    output.installed = true;
    output.verified_version = Some(verified);
    output.notes.push(format!(
        "Installed `{}`. Ensure `{}` is on PATH before older Tendril installations.",
        plan.install_path.display(),
        install_dir.display()
    ));

    fs::remove_dir_all(&temp_root).map_err(|error| io_error(&temp_root, &error))?;
    Ok(())
}

fn build_update_plan(
    repository: &str,
    version: &str,
    platform: &str,
    install_dir: &Path,
) -> UpdatePlan {
    let version = normalize_version(version);
    let tag = format!("v{version}");
    let archive_name = format!("tendril-{version}-{platform}.tar.gz");
    let checksum_name = format!("tendril-{version}-{platform}.sha256");
    let base = format!("https://github.com/{repository}/releases/download/{tag}");
    UpdatePlan {
        repository: repository.to_owned(),
        version,
        tag,
        platform: platform.to_owned(),
        archive_url: format!("{base}/{archive_name}"),
        checksum_url: format!("{base}/{checksum_name}"),
        archive_name,
        checksum_name,
        install_path: install_dir.join(release_binary_name(platform)),
    }
}

pub(crate) fn query_latest_release_version(repository: &str) -> Result<String, TendrilError> {
    if let Some(tag) = query_latest_release_version_with_gh(repository)? {
        return Ok(tag);
    }

    let url = format!("https://api.github.com/repos/{repository}/releases/latest");
    let output = ProcessCommand::new("curl")
        .args(["-fsSL", "-H", "Accept: application/vnd.github+json", &url])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "update_curl_unavailable",
                format!("failed to spawn curl to query the latest release: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(command_error(
            "update_latest_release_query_failed",
            "curl",
            &output,
        ));
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let tag = extract_json_string_field(&body, "tag_name").ok_or_else(|| {
        TendrilError::serialization("GitHub latest release response did not contain tag_name")
            .with_code("update_latest_release_missing_tag")
    })?;
    Ok(normalize_version(&tag))
}

fn query_latest_release_version_with_gh(repository: &str) -> Result<Option<String>, TendrilError> {
    let output = match ProcessCommand::new("gh")
        .args([
            "release", "view", "--repo", repository, "--json", "tagName", "--jq", ".tagName",
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(TendrilError::execution_failure(
                "update_gh_unavailable",
                format!("failed to spawn gh to query the latest release: {error}"),
                None,
            ));
        }
    };
    if output.status.success() {
        let tag = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!tag.is_empty()).then(|| normalize_version(&tag)))
    } else {
        Ok(None)
    }
}

pub(crate) fn download_asset_to_path(
    repository: &str,
    tag: &str,
    asset_name: &str,
    url: &str,
    path: &Path,
) -> Result<(), TendrilError> {
    if download_asset_with_gh(repository, tag, asset_name, path)? {
        return Ok(());
    }
    download_to_path(url, path)
}

fn download_asset_with_gh(
    repository: &str,
    tag: &str,
    asset_name: &str,
    path: &Path,
) -> Result<bool, TendrilError> {
    let output = match ProcessCommand::new("gh")
        .args([
            "release",
            "download",
            tag,
            "--repo",
            repository,
            "--pattern",
            asset_name,
            "--output",
        ])
        .arg(path)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(TendrilError::execution_failure(
                "update_gh_unavailable",
                format!("failed to spawn gh while downloading {asset_name}: {error}"),
                None,
            ));
        }
    };
    Ok(output.status.success() && path.is_file())
}

fn download_to_path(url: &str, path: &Path) -> Result<(), TendrilError> {
    let output = ProcessCommand::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(path)
        .arg(url)
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "update_curl_unavailable",
                format!("failed to spawn curl while downloading {url}: {error}"),
                None,
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error("update_download_failed", "curl", &output)
            .with_detail_entry("url", json!(url)))
    }
}

pub(crate) fn verify_checksum(
    archive_path: &Path,
    checksum_path: &Path,
) -> Result<(), TendrilError> {
    let expected_text =
        fs::read_to_string(checksum_path).map_err(|error| io_error(checksum_path, &error))?;
    let expected = expected_text.split_whitespace().next().ok_or_else(|| {
        TendrilError::validation("downloaded checksum file was empty")
            .with_code("update_empty_checksum")
    })?;
    let actual = sha256_hex(archive_path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(TendrilError::validation(
            "downloaded Tendril archive checksum did not match release checksum",
        )
        .with_code("update_checksum_mismatch")
        .with_detail_entry("expected", json!(expected))
        .with_detail_entry("actual", json!(actual)))
    }
}

fn sha256_hex(path: &Path) -> Result<String, TendrilError> {
    if let Ok(output) = ProcessCommand::new("sha256sum").arg(path).output() {
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned());
        }
    }

    let output = ProcessCommand::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "update_checksum_tool_unavailable",
                format!("failed to spawn sha256sum or shasum: {error}"),
                None,
            )
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned())
    } else {
        Err(command_error("update_checksum_failed", "shasum", &output))
    }
}

pub(crate) fn extract_archive(archive_path: &Path, extract_dir: &Path) -> Result<(), TendrilError> {
    let output = ProcessCommand::new("tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(extract_dir)
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "update_tar_unavailable",
                format!("failed to spawn tar while extracting the release archive: {error}"),
                None,
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_error("update_extract_failed", "tar", &output))
    }
}

fn verify_installed_version(binary: &Path, expected_version: &str) -> Result<String, TendrilError> {
    let output = ProcessCommand::new(binary)
        .arg("--version")
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "update_verify_failed",
                format!("failed to run installed Tendril binary: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(command_error(
            "update_verify_failed",
            &binary.display().to_string(),
            &output,
        ));
    }
    let version_text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version_text.contains(expected_version) {
        Ok(version_text)
    } else {
        Err(TendrilError::validation(
            "installed Tendril binary did not report the expected version",
        )
        .with_code("update_verify_version_mismatch")
        .with_detail_entry("expected_version", json!(expected_version))
        .with_detail_entry("actual", json!(version_text)))
    }
}

fn release_target_for(os: &str, arch: &str) -> Result<String, TendrilError> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-linux".to_owned()),
        ("linux", "aarch64" | "arm64") => Ok("aarch64-linux".to_owned()),
        ("macos", "aarch64" | "arm64") => Ok("aarch64-darwin".to_owned()),
        ("macos", "x86_64") => Ok("x86_64-darwin".to_owned()),
        ("windows", "x86_64") => Ok("x86_64-windows".to_owned()),
        ("windows", "aarch64" | "arm64") => Ok("aarch64-windows".to_owned()),
        _ => Err(TendrilError::unsupported_capability(
            "update_unsupported_platform",
            format!("Tendril release downloads are not available for {os}/{arch}"),
            Some(json!({ "os": os, "arch": arch })),
        )),
    }
}

fn default_install_dir() -> Result<PathBuf, TendrilError> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        TendrilError::config("HOME is not set; pass --install-dir explicitly")
            .with_code("update_home_not_set")
    })?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

pub(crate) fn normalize_version(version: &str) -> String {
    version.trim().trim_start_matches('v').to_owned()
}

fn release_binary_name(platform: &str) -> &'static str {
    if platform.ends_with("-windows") {
        "tendril.exe"
    } else {
        "tendril"
    }
}

fn extract_json_string_field(body: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let after_field = body.split_once(&marker)?.1;
    let after_colon = after_field.split_once(':')?.1.trim_start();
    let raw = after_colon.strip_prefix('"')?;
    let end = raw.find('"')?;
    Some(raw[..end].to_owned())
}

fn command_error(code: &'static str, command: &str, output: &std::process::Output) -> TendrilError {
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
    TendrilError::config_path(path, error.to_string()).with_code("update_io_error")
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), TendrilError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| io_error(path, &error))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| io_error(path, &error))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), TendrilError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_update_plan, extract_json_string_field, normalize_version, release_binary_name,
        release_target_for, render_update_human, verify_checksum, UpdateOutput,
    };

    #[test]
    fn maps_supported_release_targets() {
        assert_eq!(
            release_target_for("linux", "x86_64").expect("target"),
            "x86_64-linux"
        );
        assert_eq!(
            release_target_for("macos", "aarch64").expect("target"),
            "aarch64-darwin"
        );
        assert_eq!(
            release_target_for("windows", "x86_64").expect("target"),
            "x86_64-windows"
        );
        // Remaining supported pairs.
        assert_eq!(
            release_target_for("linux", "aarch64").expect("target"),
            "aarch64-linux"
        );
        assert_eq!(
            release_target_for("macos", "x86_64").expect("target"),
            "x86_64-darwin"
        );
        assert_eq!(
            release_target_for("windows", "aarch64").expect("target"),
            "aarch64-windows"
        );
        // The `arm64` spelling is an accepted alias for aarch64 on every OS.
        assert_eq!(
            release_target_for("linux", "arm64").expect("target"),
            "aarch64-linux"
        );
        assert_eq!(
            release_target_for("macos", "arm64").expect("target"),
            "aarch64-darwin"
        );
        assert_eq!(
            release_target_for("windows", "arm64").expect("target"),
            "aarch64-windows"
        );
    }

    #[test]
    fn unsupported_platform_target_is_rejected_with_os_arch_details() {
        let error = release_target_for("freebsd", "riscv64")
            .expect_err("an unknown os/arch pair should be rejected");
        assert_eq!(error.code(), "update_unsupported_platform");
        let details = error.details().expect("details");
        assert_eq!(details["os"], "freebsd");
        assert_eq!(details["arch"], "riscv64");
    }

    #[test]
    fn builds_github_release_asset_urls() {
        let plan = build_update_plan(
            "harryaskham/tendril",
            "v1.2.3",
            "x86_64-linux",
            std::path::Path::new("/tmp/bin"),
        );

        assert_eq!(plan.version, "1.2.3");
        assert_eq!(plan.tag, "v1.2.3");
        assert_eq!(plan.archive_name, "tendril-1.2.3-x86_64-linux.tar.gz");
        assert_eq!(
            plan.archive_url,
            "https://github.com/harryaskham/tendril/releases/download/v1.2.3/tendril-1.2.3-x86_64-linux.tar.gz"
        );
        assert_eq!(plan.install_path, std::path::Path::new("/tmp/bin/tendril"));

        let windows_plan = build_update_plan(
            "harryaskham/tendril",
            "v1.2.3",
            "x86_64-windows",
            std::path::Path::new("/tmp/bin"),
        );
        assert_eq!(
            windows_plan.archive_name,
            "tendril-1.2.3-x86_64-windows.tar.gz"
        );
        assert_eq!(
            windows_plan.install_path,
            std::path::Path::new("/tmp/bin/tendril.exe")
        );
    }

    #[test]
    fn parses_latest_release_tag_from_github_json() {
        let body = r#"{"name":"Release","tag_name":"v0.4.5"}"#;

        assert_eq!(
            extract_json_string_field(body, "tag_name"),
            Some("v0.4.5".to_owned())
        );
    }

    #[test]
    fn strips_optional_v_prefix_from_versions() {
        assert_eq!(normalize_version("v1.2.3"), "1.2.3");
        assert_eq!(normalize_version("1.2.3"), "1.2.3");
    }

    fn sample_update_output() -> UpdateOutput {
        UpdateOutput {
            repository: "harryaskham/tendril".to_owned(),
            version: "1.2.3".to_owned(),
            tag: "v1.2.3".to_owned(),
            platform: "x86_64-linux".to_owned(),
            archive_url: "https://example.com/tendril.tar.gz".to_owned(),
            checksum_url: "https://example.com/tendril.sha256".to_owned(),
            install_path: std::path::PathBuf::from("/tmp/bin/tendril"),
            installed: false,
            verified_version: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn renders_update_human_planned_without_notes() {
        let rendered = render_update_human(&sample_update_output());
        assert!(
            rendered.starts_with("planned Tendril v1.2.3 for x86_64-linux"),
            "unexpected header, got:\n{rendered}"
        );
        assert!(rendered.contains("install path: /tmp/bin/tendril"));
        assert!(rendered.contains("verified version: not run"));
        assert!(rendered.contains("archive: https://example.com/tendril.tar.gz"));
        assert!(rendered.contains("checksum: https://example.com/tendril.sha256"));
        assert!(
            !rendered.contains("notes:"),
            "empty notes should be omitted, got:\n{rendered}"
        );
    }

    #[test]
    fn renders_update_human_installed_with_notes() {
        let output = UpdateOutput {
            installed: true,
            verified_version: Some("1.2.3".to_owned()),
            notes: vec!["reused cached archive".to_owned(), "checksum ok".to_owned()],
            ..sample_update_output()
        };
        let rendered = render_update_human(&output);
        assert!(rendered.starts_with("installed Tendril v1.2.3 for x86_64-linux"));
        assert!(rendered.contains("verified version: 1.2.3"));
        assert!(rendered.contains("notes:\n"));
        assert!(rendered.contains("  - reused cached archive"));
        assert!(rendered.contains("  - checksum ok"));
    }

    #[test]
    fn release_binary_name_is_exe_only_for_windows_platforms() {
        assert_eq!(release_binary_name("x86_64-windows"), "tendril.exe");
        assert_eq!(release_binary_name("aarch64-windows"), "tendril.exe");
        assert_eq!(release_binary_name("x86_64-linux"), "tendril");
        assert_eq!(release_binary_name("aarch64-linux"), "tendril");
        assert_eq!(release_binary_name("aarch64-darwin"), "tendril");
        assert_eq!(release_binary_name("x86_64-darwin"), "tendril");
    }

    // sha256("tendril-test-archive") computed with `shasum -a 256`.
    const KNOWN_ARCHIVE_PAYLOAD: &[u8] = b"tendril-test-archive";
    const KNOWN_ARCHIVE_SHA256: &str =
        "d7d43dc4ce9c4d08e14948da097048df27dbba6cbd4e4648b3ebab25a2f7b05f";

    #[test]
    fn verify_checksum_accepts_matching_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("tendril.tar.gz");
        let checksum = dir.path().join("tendril.tar.gz.sha256");
        std::fs::write(&archive, KNOWN_ARCHIVE_PAYLOAD).expect("archive");
        // Standard `<hash>  <filename>` checksum line; only the first token is used.
        std::fs::write(&checksum, format!("{KNOWN_ARCHIVE_SHA256}  tendril.tar.gz\n"))
            .expect("checksum");
        verify_checksum(&archive, &checksum).expect("matching checksum should verify");
    }

    #[test]
    fn verify_checksum_rejects_mismatched_digest_with_expected_and_actual() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("tendril.tar.gz");
        let checksum = dir.path().join("tendril.tar.gz.sha256");
        std::fs::write(&archive, KNOWN_ARCHIVE_PAYLOAD).expect("archive");
        let wrong = "0".repeat(64);
        std::fs::write(&checksum, &wrong).expect("checksum");
        let error =
            verify_checksum(&archive, &checksum).expect_err("mismatched checksum is rejected");
        assert_eq!(error.code(), "update_checksum_mismatch");
        let details = error.details().expect("details");
        assert_eq!(details["expected"], wrong);
        assert_eq!(details["actual"], KNOWN_ARCHIVE_SHA256);
    }

    #[test]
    fn verify_checksum_rejects_empty_checksum_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("tendril.tar.gz");
        let checksum = dir.path().join("tendril.tar.gz.sha256");
        std::fs::write(&archive, KNOWN_ARCHIVE_PAYLOAD).expect("archive");
        std::fs::write(&checksum, "   \n").expect("checksum");
        let error =
            verify_checksum(&archive, &checksum).expect_err("empty checksum is rejected");
        assert_eq!(error.code(), "update_empty_checksum");
    }
}
