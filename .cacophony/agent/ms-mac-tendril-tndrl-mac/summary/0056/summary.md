# Session summary — sync vendored mcp-cli + fix broken-on-main runtime-dep audit

## Bead(s)
- bd-8afc79 — sync vendored crates/mcp-cli to 941015b (NDJSON framing)

## Changes
- crates/mcp-cli submodule pin: 9e2f1fc -> 941015b (matches the flake input; in-tree source no longer stale vs NDJSON contract).
- crates/tendril/tests/runtime_dependency_audit.rs: add "ffmpeg" to expected_programs (was RED on main — listen.rs avfoundation recorder bd-d92c7e spawns ffmpeg; test-small gate skipped it).

## Validation
- nix build .#tendril rc=0 (344 lib tests + runtime_dependency_audit now pass).
- nix build .#mcp-cli cold-compiled clean (timed out only on cache warm, not a failure).

## Operator-takeaway
Vendored mcp-cli matches the 941015b NDJSON pin; main is green again (ffmpeg audit fixed). Landed via PR mode.
