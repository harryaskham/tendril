# Session summary — JSON help includes update and version

## Goal

Fix Tendril's machine-readable top-level help so agents using `tendril --json` can discover the same `update` and `version` commands that are already present in the human help text.

## Bead(s)

- `bd-9f86a8` — JSON help command list omits update and version

## Before state

- Failing tests: no pre-existing failing test was run; the structured help coverage did not assert command-list parity for `update` or `version`.
- Relevant metrics: `tendril --json` exposed `list`, `list-elements`, `capture`, `run`, `clipboard`, `alias`, `listen`, and `mcp stdio`, but omitted `update` and `version`.
- Context: the human help text already advertised both commands, so the defect was isolated to `build_help_output`'s `commands` vector and its unit coverage.

## After state

- Failing tests: none observed in targeted validation.
- Relevant metrics: `target/debug/tendril --json` now reports `['list', 'list-elements', 'capture', 'run', 'clipboard', 'alias', 'listen', 'update', 'version', 'mcp stdio']` for `data.commands[].name`.
- Context: the JSON help envelope and human help text now agree on Tendril's self-update and workspace-version-management surfaces.

## Diff summary

- Code/content commits: `7e3c8df`.
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA.
- Files touched: `crates/tendril/src/commands/mod.rs`.
- Tests: +0 / -0 / flipped 0; extended an existing unit test assertion.
- Behavioural delta: structured top-level help now includes `update` and `version` command summaries.
- Validation: `cargo fmt --check`; `cargo test -p tendril commands::tests::json_help_dispatch_returns_machine_readable_envelope`; `target/debug/tendril --json` command-name inspection.

## Operator-takeaway

Agents depending on Tendril's JSON help can now discover update and version management without falling back to human help parsing.
