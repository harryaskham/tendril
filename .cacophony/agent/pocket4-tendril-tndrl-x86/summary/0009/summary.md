# Session summary — Tendril MCP: per-call x11_display override for headless/Xvfb workflows (bd-6abe70)

## Goal

Close the gap where the Tendril MCP server cannot target an X11 display brought up after it spawned. The MCP server is one long-lived process whose environment is fixed at spawn, so on a headless node that starts a virtual display (Xvfb) mid-session, the server's `$DISPLAY` is still unset and `tendril_list` fails with `platform_adapter_failure`. The direct CLI works (fresh process reads env `$DISPLAY`), so the fix is an MCP-surface per-call display override. I found this myself in this session's QA pass and filed bd-6abe70; the operator then freed capacity and invited hard work, so I implemented it.

## Bead(s)

- `bd-6abe70` — Tendril MCP server cannot target an X11 DISPLAY brought up after it spawned (headless + Xvfb agent workflows) [P3, task]

## Before state

- `x11rb::connect(None)` in `x11.rs`/`clipboard.rs` reads process `$DISPLAY`; `execute_input` built a local `AdapterContext::linux(X11, None)` ignoring any override.
- MCP tools (`list`/`capture`/`run`/`list_elements`) had no way to specify an X11 display; the server rejects top-level CLI flags in MCP mode.
- E2E repro: MCP `list` on a DISPLAY-less server → `error: failed to connect to the active X11 display: $DISPLAY variable not set`.

## After state

- `AdapterContext` gains `x11_display: Option<String>` + `with_x11_display()` builder; `X11Connection::connect` uses `x11rb::connect(context.x11_display.as_deref())`; `execute_input` threads the override from the adapter's context.
- MCP payloads: `TargetScope` (shared by capture/run/list_elements/alias) and a new `ListRequest` wrapper carry `x11_display`; each tool builds its adapter via `CommandContext::adapter_with_x11_display(...)`. Direct CLI unchanged (ambient `$DISPLAY`).
- E2E proof (Xvfb `:99`, DISPLAY-less MCP server): `list` with no override → `error/platform_adapter_failure` (reproduces bug); `list` with `x11_display=":99"` → `success`, discovers the display + window. Both CI integration tests (`mcp_external_smoke`, `mcp_parity`) pass; new unit test `x11_display_override_deserializes_and_threads_into_context` passes.
- Updated `mcp_external_smoke.rs` expected schema sets (x11_display added to list_elements/capture/run) and documented the new arg in `docs/src/mcp.md`.
- Scope note: clipboard's separate X11 connect path is left as a follow-up (still ambient `$DISPLAY`); documented in the commit.

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt (tendril uses PR-backend auto-merge on green CI).
- Files touched: `crates/tendril/src/platform.rs`, `crates/tendril/src/x11.rs`, `crates/tendril/src/clipboard.rs` (literal), `crates/tendril/src/commands/mod.rs` (payloads + helper + tool wiring + unit test), `crates/tendril/tests/mcp_external_smoke.rs` (schema sets), `docs/src/mcp.md`.
- Tests: +1 unit test; updated 3 schema-contract assertions. Validated locally: rustfmt clean, `clippy --workspace --all-targets --all-features -D warnings` clean, `cargo check --tests`, mcp integration tests, and a live Xvfb MCP e2e.
- Behavioural delta: MCP `list`/`capture`/`run`/`list_elements` accept an optional `x11_display`; omitting it preserves current ambient-`$DISPLAY` behaviour (backward compatible).

## Operator-takeaway

Tendril's MCP server is a persistent process, so anything derived from its spawn-time environment (like `$DISPLAY`) must be overridable per-call for headless/virtual-display agent workflows. The pattern is a per-tool-payload override threaded into `AdapterContext` (top-level CLI flags are deliberately rejected in MCP mode). The same shape would extend cleanly to Wayland (`WAYLAND_DISPLAY`) and to the clipboard connect path if those headless scenarios come up.
