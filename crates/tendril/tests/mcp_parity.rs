mod common;

use common::CliHarness;
use serde_json::json;

#[test]
#[allow(clippy::too_many_lines)]
fn cli_and_mcp_stdio_return_equivalent_structured_payloads() {
    let harness = CliHarness::new();

    let cli_list = harness.cli_json(&["--json", "list"]);
    let cli_capture = harness.cli_json(&["--json", "--window", "window-1", "capture"]);
    let mut cli_run = harness.cli_json(&[
        "--json",
        "--window",
        "window-1",
        "run",
        r#"send("hello parity")"#,
    ]);
    let cli_listen = harness.cli_json_lenient(&[
        "--json",
        "listen",
        "--source",
        "system",
        "--duration-ms",
        "100",
        "--format",
        "wav",
    ]);

    let responses = harness.mcp_round_trip(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "list",
                "arguments": {}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "capture",
                "arguments": {
                    "window": "window-1"
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "run",
                "arguments": {
                    "window": "window-1",
                    "input_definition": r#"send("hello parity")"#
                }
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "listen",
                "arguments": {
                    "source": "system",
                    "duration_ms": 100,
                    "format": "wav"
                }
            }
        }),
    ]);

    assert_eq!(responses.len(), 6);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "tendril");

    let tool_names = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        vec![
            "list",
            "list_elements",
            "capture",
            "run",
            "listen",
            "clipboard_get",
            "clipboard_set",
            "self_update_status",
            "self_update_check",
            "self_update_run",
            "feedback_report",
            "feedback_status"
        ]
    );

    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(responses[2]["result"]["structuredContent"], cli_list);

    assert_eq!(responses[3]["result"]["isError"], false);
    assert_eq!(responses[3]["result"]["structuredContent"], cli_capture);

    assert_eq!(responses[4]["result"]["isError"], false);
    let mcp_run = &responses[4]["result"]["structuredContent"];
    assert_eq!(mcp_run["data"]["execution_lock"]["enabled"], true);
    assert_eq!(mcp_run["data"]["execution_lock"]["acquired"], true);
    // CLI and MCP both exercise the default execution lock, but each call has
    // its own process-specific owner PID, token, and timing metadata. Normalize
    // the transient lock report before asserting the rest of the shared run
    // envelope stays byte-for-byte equivalent across surfaces.
    cli_run["data"]["execution_lock"] = mcp_run["data"]["execution_lock"].clone();
    assert_eq!(mcp_run, &cli_run);

    // The listen surface depends on a real audio backend, which may or may
    // not be available depending on the environment:
    //
    //   * In an interactive session with PipeWire/PulseAudio, both CLI and
    //     MCP succeed and emit a `data` payload with `request` + `adapter`.
    //     Artifact paths and recorder transient details may differ between
    //     the two invocations (each call allocates its own temp file and
    //     may pick a different recorder), so we compare only the request
    //     echo and adapter shape that must be deterministic.
    //
    //   * In a sandboxed Nix build with no audio backend, both CLI and MCP
    //     fail with the same `unsupported_capability` error envelope. The
    //     parity guarantee still holds: identical JSON for identical inputs.
    let cli_listen_status = cli_listen["status"].as_str().unwrap_or("");
    let mcp_listen = &responses[5]["result"]["structuredContent"];
    let mcp_listen_status = mcp_listen["status"].as_str().unwrap_or("");
    assert_eq!(
        cli_listen_status, mcp_listen_status,
        "listen parity broken: CLI status={cli_listen_status} MCP status={mcp_listen_status}\nCLI: {cli_listen}\nMCP: {mcp_listen}"
    );
    assert_eq!(
        mcp_listen["meta"]["command"], "listen",
        "MCP listen envelope must carry command=listen"
    );
    match cli_listen_status {
        "success" => {
            assert_eq!(responses[5]["result"]["isError"], false);
            let cli_listen_data = &cli_listen["data"];
            let mcp_listen_data = &mcp_listen["data"];
            assert_eq!(mcp_listen_data["request"], cli_listen_data["request"]);
            assert_eq!(mcp_listen_data["adapter"], cli_listen_data["adapter"]);
        }
        "error" => {
            // No audio backend (e.g. Nix sandbox). Both surfaces must
            // surface the same structured error so callers can branch
            // identically regardless of transport.
            assert_eq!(responses[5]["result"]["isError"], true);
            assert_eq!(cli_listen["error"]["code"], mcp_listen["error"]["code"]);
            assert_eq!(
                cli_listen["error"]["category"],
                mcp_listen["error"]["category"]
            );
        }
        other => panic!("unexpected listen envelope status: {other}\n{cli_listen}"),
    }
}
