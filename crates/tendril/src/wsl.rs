use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::json;

use crate::cli::{Command as TendrilCommand, McpSubcommand, TendrilCli};
use crate::error::TendrilError;
use crate::update;

const WINDOWS_DEFAULT_TARGET: &str = "x86_64-windows";
const WINDOWS_BIN_ENV: &str = "TENDRIL_WSL_WINDOWS_BIN";
const WINDOWS_INSTALL_DIR_ENV: &str = "TENDRIL_WSL_INSTALL_DIR";
const WINDOWS_REPOSITORY_ENV: &str = "TENDRIL_WSL_WINDOWS_REPOSITORY";
const WINDOWS_RELEASE_VERSION_ENV: &str = "TENDRIL_WSL_WINDOWS_RELEASE_VERSION";
const WINDOWS_TARGET_ENV: &str = "TENDRIL_WSL_WINDOWS_TARGET";

/// Proxy a Tendril invocation from WSL/Linux to the Windows host binary.
pub fn dispatch(cli: &TendrilCli, original_args: &[OsString]) -> Result<ExitCode, TendrilError> {
    let windows_args = strip_wsl_tunnel_args(original_args)?;
    let windows_bin = resolve_windows_tendril_bin()?;

    if is_mcp_stdio(cli) {
        return run_streaming_windows_tendril(&windows_bin, &windows_args);
    }

    run_captured_windows_tendril(&windows_bin, &windows_args, cli.json)
}

fn resolve_windows_tendril_bin() -> Result<String, TendrilError> {
    if let Some(explicit) = non_empty_env(WINDOWS_BIN_ENV) {
        return Ok(explicit);
    }

    if windows_tendril_on_path() {
        return Ok("tendril.exe".to_owned());
    }

    install_windows_tendril_from_release()
}

fn windows_tendril_on_path() -> bool {
    Command::new("tendril.exe")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn install_windows_tendril_from_release() -> Result<String, TendrilError> {
    let repository = non_empty_env(WINDOWS_REPOSITORY_ENV)
        .unwrap_or_else(|| update::DEFAULT_REPOSITORY.to_owned());
    let version = non_empty_env(WINDOWS_RELEASE_VERSION_ENV)
        .map_or_else(
            || update::query_latest_release_version(&repository),
            |version| Ok(update::normalize_version(&version)),
        )
        .map_err(|error| wsl_auto_install_error(&error))?;
    let target =
        non_empty_env(WINDOWS_TARGET_ENV).unwrap_or_else(|| WINDOWS_DEFAULT_TARGET.to_owned());
    let install_dir = default_windows_install_dir()?;
    let install_path = install_dir.join("tendril.exe");
    let marker_path = install_dir.join("tendril.version");

    if installed_marker_matches(&install_path, &marker_path, &version) {
        return Ok(install_path.display().to_string());
    }

    fs::create_dir_all(&install_dir).map_err(|error| wsl_io_error(&install_dir, &error))?;
    let temp_root = std::env::temp_dir().join(format!(
        "tendril-wsl-windows-install-{}",
        std::process::id()
    ));
    if temp_root.exists() {
        fs::remove_dir_all(&temp_root).map_err(|error| wsl_io_error(&temp_root, &error))?;
    }
    fs::create_dir_all(&temp_root).map_err(|error| wsl_io_error(&temp_root, &error))?;

    let tag = format!("v{version}");
    let archive_name = format!("tendril-{version}-{target}.tar.gz");
    let checksum_name = format!("tendril-{version}-{target}.sha256");
    let base_url = format!("https://github.com/{repository}/releases/download/{tag}");
    let archive_path = temp_root.join(&archive_name);
    let checksum_path = temp_root.join(&checksum_name);
    let extract_dir = temp_root.join("extract");
    fs::create_dir_all(&extract_dir).map_err(|error| wsl_io_error(&extract_dir, &error))?;

    update::download_asset_to_path(
        &repository,
        &tag,
        &archive_name,
        &format!("{base_url}/{archive_name}"),
        &archive_path,
    )
    .map_err(|error| wsl_auto_install_error(&error))?;
    update::download_asset_to_path(
        &repository,
        &tag,
        &checksum_name,
        &format!("{base_url}/{checksum_name}"),
        &checksum_path,
    )
    .map_err(|error| wsl_auto_install_error(&error))?;
    update::verify_checksum(&archive_path, &checksum_path)
        .map_err(|error| wsl_auto_install_error(&error))?;
    update::extract_archive(&archive_path, &extract_dir)
        .map_err(|error| wsl_auto_install_error(&error))?;

    let extracted_binary = extract_dir
        .join(format!("tendril-{version}-{target}"))
        .join("tendril.exe");
    if !extracted_binary.is_file() {
        return Err(TendrilError::target_not_found(
            "Windows Tendril release binary",
            extracted_binary.display().to_string(),
        )
        .with_code("wsl_tunnel_windows_release_binary_missing"));
    }

    fs::copy(&extracted_binary, &install_path)
        .map_err(|error| wsl_io_error(&install_path, &error))?;
    make_wsl_executable(&install_path)?;
    fs::write(&marker_path, format!("{version}\n"))
        .map_err(|error| wsl_io_error(&marker_path, &error))?;
    fs::remove_dir_all(&temp_root).map_err(|error| wsl_io_error(&temp_root, &error))?;

    Ok(install_path.display().to_string())
}

fn default_windows_install_dir() -> Result<PathBuf, TendrilError> {
    if let Some(path) = non_empty_env(WINDOWS_INSTALL_DIR_ENV) {
        return Ok(PathBuf::from(path));
    }

    let local_app_data = windows_local_app_data_path()?;
    Ok(local_app_data.join("Tendril").join("bin"))
}

fn windows_local_app_data_path() -> Result<PathBuf, TendrilError> {
    let output = Command::new("cmd.exe")
        .args(["/C", "echo %LOCALAPPDATA%"])
        .output()
        .map_err(|error| {
            TendrilError::execution_failure(
                "wsl_tunnel_local_app_data_unavailable",
                format!("failed to query Windows %LOCALAPPDATA% through cmd.exe: {error}"),
                None,
            )
        })?;
    if !output.status.success() {
        return Err(TendrilError::execution_failure(
            "wsl_tunnel_local_app_data_unavailable",
            format!(
                "cmd.exe failed while querying %LOCALAPPDATA%: {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            None,
        ));
    }
    let windows_path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    windows_path_to_wsl_path(&windows_path)
}

fn windows_path_to_wsl_path(windows_path: &str) -> Result<PathBuf, TendrilError> {
    if let Ok(output) = Command::new("wslpath").args(["-u", windows_path]).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }

    manual_windows_path_to_wsl_path(windows_path).ok_or_else(|| {
        TendrilError::config(format!(
            "could not convert Windows path `{windows_path}` to a WSL path; set {WINDOWS_INSTALL_DIR_ENV} explicitly"
        ))
        .with_code("wsl_tunnel_windows_path_conversion_failed")
    })
}

fn manual_windows_path_to_wsl_path(windows_path: &str) -> Option<PathBuf> {
    let normalized = windows_path.trim().trim_end_matches(['\r', '\n']);
    let bytes = normalized.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let rest = normalized[3..].replace('\\', "/");
    Some(PathBuf::from(format!("/mnt/{drive}/{rest}")))
}

fn installed_marker_matches(install_path: &Path, marker_path: &Path, version: &str) -> bool {
    install_path.is_file()
        && fs::read_to_string(marker_path)
            .is_ok_and(|text| update::normalize_version(&text) == version)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn wsl_auto_install_error(error: &TendrilError) -> TendrilError {
    TendrilError::execution_failure(
        "wsl_tunnel_windows_binary_auto_install_failed",
        format!(
            "failed to auto-install Windows Tendril binary for --wsl-tunnel from the GitHub release: {error}"
        ),
        None,
    )
}

fn wsl_io_error(path: &Path, error: &std::io::Error) -> TendrilError {
    TendrilError::config_path(path, error.to_string())
        .with_code("wsl_tunnel_windows_install_io_error")
}

#[cfg(unix)]
fn make_wsl_executable(path: &Path) -> Result<(), TendrilError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| wsl_io_error(path, &error))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| wsl_io_error(path, &error))
}

#[cfg(not(unix))]
fn make_wsl_executable(_path: &Path) -> Result<(), TendrilError> {
    Ok(())
}

fn run_streaming_windows_tendril(
    windows_bin: &str,
    args: &[String],
) -> Result<ExitCode, TendrilError> {
    let status = Command::new(windows_bin)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| windows_spawn_error(windows_bin, &error))?;
    Ok(exit_code_from_status(status))
}

fn run_captured_windows_tendril(
    windows_bin: &str,
    args: &[String],
    json_mode: bool,
) -> Result<ExitCode, TendrilError> {
    let output = Command::new(windows_bin)
        .args(args)
        .output()
        .map_err(|error| windows_spawn_error(windows_bin, &error))?;

    if !output.status.success() && should_wrap_windows_failure(json_mode, &output.stdout) {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(TendrilError::execution_failure(
            "wsl_tunnel_command_failed",
            windows_failure_message(windows_bin, output.status.code(), &stderr),
            None,
        )
        .with_detail_entry("windows_binary", json!(windows_bin))
        .with_detail_entry("exit_status", json!(output.status.code()))
        .with_detail_entry("stderr", json!(stderr)));
    }

    io::stdout().write_all(&output.stdout).map_err(|error| {
        TendrilError::execution_failure("wsl_tunnel_stdout_failed", error.to_string(), None)
    })?;
    io::stderr().write_all(&output.stderr).map_err(|error| {
        TendrilError::execution_failure("wsl_tunnel_stderr_failed", error.to_string(), None)
    })?;
    Ok(exit_code_from_status(output.status))
}

pub(crate) fn strip_wsl_tunnel_args(args: &[OsString]) -> Result<Vec<String>, TendrilError> {
    let mut stripped = Vec::new();
    for arg in args.iter().skip(1) {
        let value = arg.to_str().ok_or_else(|| {
            TendrilError::validation("--wsl-tunnel proxy arguments must be valid UTF-8")
                .with_code("invalid_wsl_tunnel_input")
                .with_field("argv")
        })?;
        if value == "--wsl-tunnel" {
            continue;
        }
        stripped.push(value.to_owned());
    }
    Ok(stripped)
}

fn should_wrap_windows_failure(json_mode: bool, stdout: &[u8]) -> bool {
    json_mode && stdout.is_empty()
}

fn windows_spawn_error(windows_bin: &str, error: &std::io::Error) -> TendrilError {
    TendrilError::execution_failure(
        "wsl_tunnel_windows_binary_spawn_failed",
        format!(
            "failed to start Windows Tendril binary `{windows_bin}` from WSL tunnel: {error}. Tendril tried TENDRIL_WSL_WINDOWS_BIN, tendril.exe on PATH, and auto-installing the latest Windows release into %LOCALAPPDATA%\\Tendril\\bin. Set {WINDOWS_BIN_ENV} or {WINDOWS_INSTALL_DIR_ENV} when the Windows executable lives somewhere else."
        ),
        None,
    )
    .with_detail_entry("windows_binary", json!(windows_bin))
}

fn windows_failure_message(windows_bin: &str, status_code: Option<i32>, stderr: &str) -> String {
    let status = status_code.map_or_else(
        || "terminated by signal".to_owned(),
        |code| code.to_string(),
    );
    if stderr.is_empty() {
        format!("Windows Tendril binary `{windows_bin}` failed with exit status {status}")
    } else {
        format!("Windows Tendril binary `{windows_bin}` failed with exit status {status}: {stderr}")
    }
}

fn exit_code_from_status(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::from(1),
    }
}

fn is_mcp_stdio(cli: &TendrilCli) -> bool {
    matches!(
        &cli.command,
        Some(TendrilCommand::Mcp(crate::cli::McpCommand {
            command: McpSubcommand::Stdio,
        }))
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{installed_marker_matches, manual_windows_path_to_wsl_path, strip_wsl_tunnel_args};

    #[test]
    fn strips_wsl_tunnel_flag_and_preserves_remaining_arguments() {
        let args = [
            OsString::from("tendril"),
            OsString::from("--wsl-tunnel"),
            OsString::from("--json"),
            OsString::from("--window"),
            OsString::from("0x1234"),
            OsString::from("run"),
            OsString::from(r#"send("hello, windows")"#),
        ];

        let stripped = strip_wsl_tunnel_args(&args).expect("valid argv");

        assert_eq!(
            stripped,
            vec![
                "--json",
                "--window",
                "0x1234",
                "run",
                r#"send("hello, windows")"#,
            ]
        );
    }

    #[test]
    fn converts_windows_local_app_data_path_to_wsl_mount_path() {
        assert_eq!(
            manual_windows_path_to_wsl_path(r"C:\Users\Agent\AppData\Local"),
            Some(PathBuf::from("/mnt/c/Users/Agent/AppData/Local"))
        );
        assert_eq!(manual_windows_path_to_wsl_path(r"\\server\share"), None);
    }

    #[test]
    fn installed_marker_requires_matching_version_and_exe() {
        let temp = tempfile::tempdir().expect("tempdir");
        let exe = temp.path().join("tendril.exe");
        let marker = temp.path().join("tendril.version");
        std::fs::write(&marker, "1.2.3\n").expect("marker");
        assert!(!installed_marker_matches(&exe, &marker, "1.2.3"));

        std::fs::write(&exe, b"fake exe").expect("exe");
        assert!(installed_marker_matches(&exe, &marker, "1.2.3"));
        assert!(!installed_marker_matches(&exe, &marker, "1.2.4"));
    }
}
