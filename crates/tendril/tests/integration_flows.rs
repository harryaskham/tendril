mod common;

use std::process::Command;

use common::CliHarness;

#[test]
fn list_capture_and_run_flow_uses_isolated_config_and_dynamic_fixtures() {
    let harness = CliHarness::new();

    let list_json = harness.cli_json(&["--json", "list"]);
    assert_eq!(list_json["status"], "success");
    assert_eq!(list_json["meta"]["command"], "list");
    assert_eq!(
        list_json["data"]["targets"]
            .as_array()
            .expect("targets array")
            .len(),
        2
    );
    assert_eq!(list_json["data"]["targets"][0]["id"], "1");
    assert_eq!(list_json["data"]["targets"][1]["id"], "window-1");

    let capture_json = harness.cli_json(&["--json", "--window", "window-1", "capture"]);
    assert_eq!(capture_json["status"], "success");
    assert_eq!(capture_json["meta"]["command"], "capture");
    assert_eq!(capture_json["data"]["target"]["id"], "window-1");
    assert_eq!(capture_json["data"]["format"], "jpeg");
    assert_eq!(capture_json["data"]["compression"], 72);
    assert_eq!(capture_json["data"]["media_type"], "image/jpeg");
    assert_eq!(capture_json["data"]["original_bounds"]["width"], 4);
    assert_eq!(capture_json["data"]["original_bounds"]["height"], 2);
    assert_eq!(capture_json["data"]["output_bounds"]["width"], 2);
    assert_eq!(capture_json["data"]["output_bounds"]["height"], 1);
    assert_eq!(capture_json["data"]["output_to_source"]["x_numerator"], 4);
    assert_eq!(capture_json["data"]["output_to_source"]["x_denominator"], 2);
    assert_eq!(capture_json["data"]["captured_at"], "2026-04-09T18:00:00Z");

    let run_json = harness.cli_json(&[
        "--json",
        "--window",
        "window-1",
        "run",
        r#"send("hello fixture")"#,
    ]);
    assert_eq!(run_json["status"], "success");
    assert_eq!(run_json["meta"]["command"], "run");
    assert_eq!(run_json["data"]["target"]["id"], "window-1");
    assert_eq!(run_json["data"]["action_count"], 1);
    assert_eq!(run_json["data"]["focus_required"], true);
    assert_eq!(run_json["data"]["focus_transferred"], true);
    assert_eq!(run_json["data"]["focused_target"], "window-1");
    assert_eq!(run_json["data"]["previous_focus"]["id"], "previous-window");
    assert_eq!(run_json["data"]["focus_restored"], true);
    assert_eq!(run_json["data"]["pointer_restored"], true);
    assert_eq!(run_json["data"]["notes"][0], "fixture input executed");
}

#[test]
#[cfg(unix)]
fn remote_run_proxies_over_ssh_and_preserves_quoted_arguments() {
    use std::os::unix::fs::PermissionsExt as _;

    let harness = CliHarness::new();
    let fake_bin = tempfile::tempdir().expect("fake bin tempdir");
    let log_path = fake_bin.path().join("ssh-target.log");
    let ssh_path = fake_bin.path().join("ssh");
    let tendril_path = fake_bin.path().join("tendril");
    let real_tendril = env!("CARGO_BIN_EXE_tendril");

    std::fs::write(
        &ssh_path,
        "#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$TENDRIL_FAKE_SSH_TARGET_LOG\"\nshift\nexec /bin/sh -c \"$1\"\n",
    )
    .expect("fake ssh script");
    std::fs::write(
        &tendril_path,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            real_tendril.replace('\'', "'\\''")
        ),
    )
    .expect("fake tendril shim");
    std::fs::set_permissions(&ssh_path, std::fs::Permissions::from_mode(0o755))
        .expect("fake ssh executable");
    std::fs::set_permissions(&tendril_path, std::fs::Permissions::from_mode(0o755))
        .expect("fake tendril executable");

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(fake_bin.path().to_path_buf()).chain(std::env::split_paths(&old_path)),
    )
    .expect("joined PATH");

    let mut command = Command::new(real_tendril);
    harness.apply_env(&mut command);
    let output = command
        .env("PATH", path)
        .env("TENDRIL_FAKE_SSH_TARGET_LOG", &log_path)
        .args([
            "--remote",
            "me@box",
            "--json",
            "--window",
            "window-1",
            "run",
            r#"send("hello, remote")"#,
        ])
        .output()
        .expect("remote CLI should run");

    assert!(
        output.status.success(),
        "remote CLI failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("remote output should be JSON");
    assert_eq!(json["status"], "success");
    assert_eq!(json["meta"]["command"], "run");
    assert_eq!(json["data"]["target"]["id"], "window-1");
    assert_eq!(json["data"]["notes"][0], "fixture input executed");
    assert_eq!(
        std::fs::read_to_string(log_path).expect("ssh target log"),
        "me@box\n"
    );
}

#[test]
#[cfg(unix)]
fn remote_ssh_failures_return_structured_json_errors() {
    use std::os::unix::fs::PermissionsExt as _;

    let harness = CliHarness::new();
    let fake_bin = tempfile::tempdir().expect("fake bin tempdir");
    let ssh_path = fake_bin.path().join("ssh");
    let real_tendril = env!("CARGO_BIN_EXE_tendril");

    std::fs::write(
        &ssh_path,
        "#!/bin/sh\necho 'ssh: connect to host badhost port 22: No route to host' >&2\nexit 255\n",
    )
    .expect("fake ssh script");
    std::fs::set_permissions(&ssh_path, std::fs::Permissions::from_mode(0o755))
        .expect("fake ssh executable");

    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(fake_bin.path().to_path_buf()).chain(std::env::split_paths(&old_path)),
    )
    .expect("joined PATH");

    let mut command = Command::new(real_tendril);
    harness.apply_env(&mut command);
    let output = command
        .env("PATH", path)
        .args(["--json", "--remote", "me@badhost", "list"])
        .output()
        .expect("remote CLI should run");

    assert!(!output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("remote failure should be JSON");
    assert_eq!(json["status"], "error");
    assert_eq!(json["meta"]["command"], "list");
    assert_eq!(json["error"]["code"], "remote_ssh_failed");
    assert_eq!(json["error"]["details"]["remote"], "me@badhost");
    assert!(
        json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("No route to host")
    );
}
