use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::{Command, ExitCode, Stdio};

use serde_json::json;

use crate::cli::{Command as TendrilCommand, McpSubcommand, TendrilCli};
use crate::error::TendrilError;

/// Proxy a parsed Tendril invocation to a remote host over OpenSSH.
///
/// The local CLI does not attempt to interpret target/window/platform details
/// for the remote host. Instead it strips only the local `--remote` flag,
/// bootstraps a graphical desktop environment in the remote shell, and then
/// execs `tendril` on that host with the original command arguments.
pub fn dispatch(cli: &TendrilCli, original_args: &[OsString]) -> Result<ExitCode, TendrilError> {
    let remote = cli
        .remote
        .as_deref()
        .ok_or_else(|| TendrilError::validation("remote dispatch requires --remote"))?;
    let remote_args = strip_remote_args(original_args)?;
    let remote_command = build_remote_shell_command(&remote_args);

    if is_mcp_stdio(cli) {
        return run_streaming_ssh(remote, &remote_command);
    }

    run_captured_ssh(remote, &remote_command, cli.json)
}

fn run_streaming_ssh(remote: &str, remote_command: &str) -> Result<ExitCode, TendrilError> {
    let status = Command::new("ssh")
        .arg(remote)
        .arg(remote_command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| remote_spawn_error(remote, &error))?;

    Ok(exit_code_from_status(status))
}

fn run_captured_ssh(
    remote: &str,
    remote_command: &str,
    json_mode: bool,
) -> Result<ExitCode, TendrilError> {
    let output = Command::new("ssh")
        .arg(remote)
        .arg(remote_command)
        .output()
        .map_err(|error| remote_spawn_error(remote, &error))?;

    if !output.status.success()
        && should_wrap_remote_failure(json_mode, output.status.code(), &output.stdout)
    {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let code = if output.status.code() == Some(255) {
            "remote_ssh_failed"
        } else {
            "remote_command_failed"
        };
        return Err(TendrilError::execution_failure(
            code,
            remote_failure_message(remote, output.status.code(), &stderr),
            None,
        )
        .with_detail_entry("remote", json!(remote))
        .with_detail_entry("exit_status", json!(output.status.code()))
        .with_detail_entry("stderr", json!(stderr)));
    }

    io::stdout().write_all(&output.stdout).map_err(|error| {
        TendrilError::execution_failure("remote_stdout_failed", error.to_string(), None)
    })?;
    io::stderr().write_all(&output.stderr).map_err(|error| {
        TendrilError::execution_failure("remote_stderr_failed", error.to_string(), None)
    })?;

    Ok(exit_code_from_status(output.status))
}

fn should_wrap_remote_failure(json_mode: bool, status_code: Option<i32>, stdout: &[u8]) -> bool {
    status_code == Some(255) || (json_mode && stdout.is_empty())
}

fn remote_spawn_error(remote: &str, error: &std::io::Error) -> TendrilError {
    TendrilError::execution_failure(
        "remote_ssh_spawn_failed",
        format!("failed to start ssh for remote `{remote}`: {error}"),
        None,
    )
    .with_detail_entry("remote", json!(remote))
}

fn remote_failure_message(remote: &str, status_code: Option<i32>, stderr: &str) -> String {
    let status = status_code.map_or_else(
        || "terminated by signal".to_owned(),
        |code| code.to_string(),
    );
    if stderr.is_empty() {
        format!("remote `{remote}` failed over ssh with exit status {status}")
    } else {
        format!("remote `{remote}` failed over ssh with exit status {status}: {stderr}")
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

pub(crate) fn strip_remote_args(args: &[OsString]) -> Result<Vec<String>, TendrilError> {
    let mut stripped = Vec::new();
    let mut skip_next = false;

    for arg in args.iter().skip(1) {
        let value = arg.to_str().ok_or_else(|| {
            TendrilError::validation("--remote proxy arguments must be valid UTF-8")
                .with_code("invalid_remote_input")
                .with_field("argv")
        })?;

        if skip_next {
            skip_next = false;
            continue;
        }

        if value == "--remote" {
            skip_next = true;
            continue;
        }
        if value.starts_with("--remote=") {
            continue;
        }

        stripped.push(value.to_owned());
    }

    Ok(stripped)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_remote_shell_command(args: &[String]) -> String {
    let rendered_args = args
        .iter()
        .map(|argument| quote_posix(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let argv_suffix = if rendered_args.is_empty() {
        String::new()
    } else {
        format!(" {rendered_args}")
    };

    format!(
        r#"set -eu
# Make common package-manager locations visible for non-login SSH shells.
case "$(uname -s 2>/dev/null || echo unknown)" in
  Darwin*)
    PATH="/opt/homebrew/bin:/usr/local/bin:/run/current-system/sw/bin:/usr/bin:/bin:${{PATH:-}}"
    export PATH
    ;;
  Linux*)
    PATH="/run/current-system/sw/bin:/usr/local/bin:/usr/bin:/bin:${{PATH:-}}"
    export PATH

    # SSH X forwarding points back at the client, not at the remote desktop.
    # `--remote` is specifically about controlling the remote host, so ignore
    # forwarded DISPLAY values and discover the host-local desktop instead.
    case "${{DISPLAY:-}}" in
      localhost:*|127.0.0.1:*|::1:*) unset DISPLAY ;;
    esac

    uid="$(id -u 2>/dev/null || true)"
    runtime="${{XDG_RUNTIME_DIR:-}}"
    if [ -z "$runtime" ] && [ -n "$uid" ] && [ -d "/run/user/$uid" ]; then
      XDG_RUNTIME_DIR="/run/user/$uid"
      runtime="$XDG_RUNTIME_DIR"
      export XDG_RUNTIME_DIR
    fi
    if [ -n "$runtime" ] && [ -z "${{DBUS_SESSION_BUS_ADDRESS:-}}" ] && [ -S "$runtime/bus" ]; then
      DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus"
      export DBUS_SESSION_BUS_ADDRESS
    fi

    needs_session_discovery=no
    case "${{XDG_SESSION_TYPE:-}}" in
      ""|tty|unknown) needs_session_discovery=yes ;;
      x11) [ -z "${{DISPLAY:-}}" ] && needs_session_discovery=yes ;;
      wayland) [ -z "${{WAYLAND_DISPLAY:-}}" ] && needs_session_discovery=yes ;;
    esac

    # Prefer systemd-logind's active graphical session when SSH did not provide
    # a complete graphical environment. This disambiguates hosts with both
    # stale X11 sockets and a live Wayland seat without overriding explicit env.
    if [ "$needs_session_discovery" = yes ] && command -v loginctl >/dev/null 2>&1; then
      user="$(id -un 2>/dev/null || true)"
      chosen="$(loginctl list-sessions --no-legend 2>/dev/null | awk -v user="$user" '$3 == user {{ print $1 }}' | while IFS= read -r sid; do
        [ -n "$sid" ] || continue
        typ="$(loginctl show-session "$sid" -p Type --value 2>/dev/null || true)"
        state="$(loginctl show-session "$sid" -p State --value 2>/dev/null || true)"
        remote="$(loginctl show-session "$sid" -p Remote --value 2>/dev/null || true)"
        case "$typ" in x11|wayland) ;; *) continue ;; esac
        [ "$remote" = "yes" ] && continue
        case "$state" in active|online) printf '%s:%s\n' "$sid" "$typ"; break ;; esac
      done)"
      if [ -n "$chosen" ]; then
        sid="${{chosen%%:*}}"
        typ="${{chosen#*:}}"
        XDG_SESSION_TYPE="$typ"
        export XDG_SESSION_TYPE
        display_value="$(loginctl show-session "$sid" -p Display --value 2>/dev/null || true)"
        if [ "$typ" = x11 ] && [ -n "$display_value" ] && [ -z "${{DISPLAY:-}}" ]; then
          DISPLAY="$display_value"
          export DISPLAY
        fi
        if [ "$typ" = wayland ] && [ -n "$display_value" ] && [ -z "${{WAYLAND_DISPLAY:-}}" ]; then
          WAYLAND_DISPLAY="$display_value"
          export WAYLAND_DISPLAY
        fi
      fi
    fi

    if [ -n "$runtime" ] && [ -z "${{WAYLAND_DISPLAY:-}}" ]; then
      for sock in "$runtime"/wayland-*; do
        [ -S "$sock" ] || continue
        WAYLAND_DISPLAY="${{sock##*/}}"
        export WAYLAND_DISPLAY
        break
      done
    fi
    if [ -z "${{DISPLAY:-}}" ]; then
      for sock in /tmp/.X11-unix/X*; do
        [ -S "$sock" ] || continue
        display_num="${{sock##*/X}}"
        [ -n "$display_num" ] || continue
        DISPLAY=":$display_num"
        export DISPLAY
        break
      done
    fi
    if [ -z "${{XDG_SESSION_TYPE:-}}" ] || [ "${{XDG_SESSION_TYPE:-}}" = tty ]; then
      if [ -n "${{WAYLAND_DISPLAY:-}}" ]; then
        XDG_SESSION_TYPE=wayland
        export XDG_SESSION_TYPE
      elif [ -n "${{DISPLAY:-}}" ]; then
        XDG_SESSION_TYPE=x11
        export XDG_SESSION_TYPE
      fi
    fi
    ;;
esac

remote_bin="${{TENDRIL_REMOTE_BIN:-tendril}}"
if ! command -v "$remote_bin" >/dev/null 2>&1; then
  echo "tendril --remote: remote tendril binary '$remote_bin' was not found on PATH; install tendril remotely or set TENDRIL_REMOTE_BIN" >&2
  exit 127
fi
exec "$remote_bin"{argv_suffix}
"#
    )
}

fn quote_posix(argument: &str) -> String {
    if !argument.is_empty() && argument.chars().all(is_posix_safe_character) {
        return argument.to_owned();
    }
    format!("'{}'", argument.replace('\'', "'\"'\"'"))
}

fn is_posix_safe_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '-' | '_' | '.' | '/' | ':' | '=' | ',')
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{build_remote_shell_command, strip_remote_args};

    #[test]
    fn strips_remote_flag_and_preserves_remaining_arguments() {
        let args = [
            OsString::from("tendril"),
            OsString::from("--json"),
            OsString::from("--remote"),
            OsString::from("me@box"),
            OsString::from("--window"),
            OsString::from("window 1"),
            OsString::from("run"),
            OsString::from(r#"send("hello, remote")"#),
        ];

        let stripped = strip_remote_args(&args).expect("valid argv");

        assert_eq!(
            stripped,
            vec![
                "--json",
                "--window",
                "window 1",
                "run",
                r#"send("hello, remote")"#
            ]
        );
    }

    #[test]
    fn strips_remote_equals_form() {
        let args = [
            OsString::from("tendril"),
            OsString::from("--remote=me@box"),
            OsString::from("list"),
        ];

        let stripped = strip_remote_args(&args).expect("valid argv");

        assert_eq!(stripped, vec!["list"]);
    }

    #[test]
    fn remote_command_bootstraps_desktop_environment_and_quotes_args() {
        let script = build_remote_shell_command(&[
            "--window".to_owned(),
            "window 1".to_owned(),
            "run".to_owned(),
            r#"send("hi, there")"#.to_owned(),
        ]);

        assert!(script.contains("XDG_RUNTIME_DIR"));
        assert!(script.contains("WAYLAND_DISPLAY"));
        assert!(script.contains("/tmp/.X11-unix/X*"));
        assert!(
            script.contains("exec \"$remote_bin\" --window 'window 1' run 'send(\"hi, there\")'")
        );
        assert!(!script.contains("exec \"$remote_bin\" --remote"));
    }
}
