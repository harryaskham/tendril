use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::{Command, ExitCode, Stdio};

use serde_json::json;

use crate::cli::{Command as TendrilCommand, McpSubcommand, TendrilCli};
use crate::error::TendrilError;
const WINDOWS_BIN_ENV: &str = "TENDRIL_WSL_WINDOWS_BIN";

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

    Err(TendrilError::unsupported_capability(
        "wsl_tunnel_windows_binary_unavailable",
        "WSL tunnel mode could not find `tendril.exe`; install Tendril on the Windows host and put it on PATH, or set TENDRIL_WSL_WINDOWS_BIN to its Windows-visible executable path",
        Some(json!({
            "environment_variable": WINDOWS_BIN_ENV,
            "searched": ["tendril.exe on Windows PATH"],
        })),
    ))
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

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
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
            "failed to start Windows Tendril binary `{windows_bin}` from WSL tunnel: {error}. Install tendril.exe on the Windows PATH or set {WINDOWS_BIN_ENV} to its Windows-visible executable path."
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

    use super::{should_wrap_windows_failure, strip_wsl_tunnel_args, windows_failure_message};

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
    fn should_wrap_windows_failure_only_for_empty_json_stdout() {
        assert!(should_wrap_windows_failure(true, b""));
        assert!(!should_wrap_windows_failure(true, b"{}"));
        assert!(!should_wrap_windows_failure(false, b""));
        assert!(!should_wrap_windows_failure(false, b"output"));
    }

    #[test]
    fn windows_failure_message_describes_signal_status_and_stderr() {
        assert_eq!(
            windows_failure_message("tendril.exe", None, ""),
            "Windows Tendril binary `tendril.exe` failed with exit status terminated by signal"
        );
        assert_eq!(
            windows_failure_message("tendril.exe", Some(3), ""),
            "Windows Tendril binary `tendril.exe` failed with exit status 3"
        );
        assert_eq!(
            windows_failure_message("tendril.exe", Some(3), "boom"),
            "Windows Tendril binary `tendril.exe` failed with exit status 3: boom"
        );
    }
}
