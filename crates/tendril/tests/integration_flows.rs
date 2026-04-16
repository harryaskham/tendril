mod common;

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
    assert_eq!(run_json["data"]["notes"][0], "fixture input executed");
}
