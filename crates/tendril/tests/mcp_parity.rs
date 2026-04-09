mod common;

use common::CliHarness;
use serde_json::json;

#[test]
fn cli_and_mcp_stdio_return_equivalent_structured_payloads() {
    let harness = CliHarness::new();

    let cli_list = harness.cli_json(&["--json", "list"]);
    let cli_capture = harness.cli_json(&["--json", "--window", "window-1", "capture"]);
    let cli_run = harness.cli_json(&[
        "--json",
        "--window",
        "window-1",
        "run",
        r#"send("hello parity")"#,
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
    ]);

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "tendril");

    let tool_names = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(tool_names, vec!["list", "capture", "run"]);

    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(responses[2]["result"]["structuredContent"], cli_list);

    assert_eq!(responses[3]["result"]["isError"], false);
    assert_eq!(responses[3]["result"]["structuredContent"], cli_capture);

    assert_eq!(responses[4]["result"]["isError"], false);
    assert_eq!(responses[4]["result"]["structuredContent"], cli_run);
}
