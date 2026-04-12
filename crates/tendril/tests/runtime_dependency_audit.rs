use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[test]
fn runtime_dependency_audit_mentions_every_spawned_program() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest should live under the workspace root");

    let mut programs = BTreeSet::new();
    for relative_path in [
        "crates/tendril/src/discovery.rs",
        "crates/tendril/src/platform.rs",
    ] {
        let source = fs::read_to_string(repo_root.join(relative_path))
            .unwrap_or_else(|error| panic!("failed to read {relative_path}: {error}"));
        collect_string_literals_after(&source, "Command::new(\"", &mut programs);
        collect_string_literals_after(&source, "run_command(context, \"", &mut programs);
        collect_string_literals_after(&source, "run_optional_command(context, \"", &mut programs);
        collect_string_literals_after(&source, "run_process_for_input(\"", &mut programs);
    }

    let expected_programs = [
        "grim",
        "hyprctl",
        "import",
        "osascript",
        "powershell",
        "screencapture",
        "swift",
        "swaymsg",
        "wlr-randr",
        "xdotool",
        "xprop",
        "xrandr",
        "xwininfo",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();

    assert_eq!(programs, expected_programs);
}

fn collect_string_literals_after(source: &str, marker: &str, programs: &mut BTreeSet<String>) {
    let mut start = 0;
    while let Some(index) = source[start..].find(marker) {
        let literal_start = start + index + marker.len();
        let Some(literal_end) = source[literal_start..].find('"') else {
            break;
        };
        programs.insert(source[literal_start..literal_start + literal_end].to_owned());
        start = literal_start + literal_end + 1;
    }
}
