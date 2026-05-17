mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::{CliHarness, frame_request, parse_framed_responses};
use serde_json::{Value, json};

#[test]
#[allow(clippy::too_many_lines)]
fn external_client_smoke_script_verifies_stdio_contract_against_built_binary() {
    let harness = CliHarness::new();
    let requests = [
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
                "name": "self_update_status",
                "arguments": {}
            }
        }),
    ];

    let mut command = Command::new(env!("CARGO_BIN_EXE_tendril"));
    command
        .args(["mcp", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    harness.apply_env(&mut command);

    let mut child = command.spawn().expect("mcp stdio should start");
    {
        let mut stdin = child.stdin.take().expect("stdin should be piped");
        for request in requests {
            stdin
                .write_all(&frame_request(&request))
                .expect("request frame should be written");
        }
    }

    let output = child
        .wait_with_output()
        .expect("mcp stdio should exit cleanly");
    assert!(
        output.status.success(),
        "mcp stdio failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let responses = parse_framed_responses(&output.stdout);
    assert_eq!(responses.len(), 4);

    let initialize = &responses[0];
    assert_eq!(initialize["result"]["serverInfo"]["name"], "tendril");

    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list should return a tools array");
    let tool_names = tools
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
            "self_update_run"
        ]
    );

    let list_elements_tool = tool(tools, "list_elements");
    let capture_tool = tool(tools, "capture");
    let run_tool = tool(tools, "run");
    let listen_tool = tool(tools, "listen");

    let list_elements_properties = property_names(&list_elements_tool["inputSchema"]);
    assert_eq!(
        list_elements_properties,
        vec!["display", "include_offscreen", "window"]
    );

    let capture_properties = property_names(&capture_tool["inputSchema"]);
    assert_eq!(
        capture_properties,
        vec![
            "compression",
            "display",
            "format",
            "max_height",
            "max_width",
            "timeout_ms",
            "window",
        ]
    );

    let run_properties = property_names(&run_tool["inputSchema"]);
    assert_eq!(
        run_properties,
        vec![
            "display",
            "input_definition",
            "lock_path",
            "lock_stale_ms",
            "lock_timeout_ms",
            "no_lock",
            "no_restore_focus",
            "restore_focus",
            "window",
        ]
    );

    let listen_properties = property_names(&listen_tool["inputSchema"]);
    assert_eq!(
        listen_properties,
        vec!["duration_ms", "format", "output", "source"]
    );

    let structured = &responses[2]["result"]["structuredContent"];
    assert_eq!(structured["meta"]["command"], "list");

    match structured["status"].as_str() {
        Some("success") => assert!(structured["data"]["targets"].is_array()),
        Some("error") => {
            assert!(structured["error"]["category"].is_string());
            assert!(structured["error"]["code"].is_string());
            assert!(structured["error"]["message"].is_string());
        }
        other => panic!("unexpected tools/call(list) status: {other:?}\nresponse: {structured}"),
    }

    let update_status = &responses[3]["result"]["structuredContent"];
    assert_eq!(responses[3]["result"]["isError"], false);
    assert_eq!(update_status["meta"]["command"], "self_update_status");
    assert_eq!(update_status["data"]["tool"], "tendril");
    assert_eq!(
        update_status["data"]["current_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        update_status["data"]["installed_path"]
            .as_str()
            .expect("installed path")
            .ends_with("/tendril")
    );
    assert!(
        update_status["data"]["next_path"]
            .as_str()
            .expect("next path")
            .ends_with("/tendril_next")
    );
}

fn tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
    tools
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("expected `{name}` tool in {tools:?}"))
}

fn property_names(schema: &Value) -> Vec<&str> {
    let mut names = schema["properties"]
        .as_object()
        .expect("tool schema should expose properties")
        .keys()
        .map(std::string::String::as_str)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}
