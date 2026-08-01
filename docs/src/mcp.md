# MCP guide

Tendril exposes its initial command surface over stdio with:

```bash
tendril mcp stdio
```

## Initial tool set

The current MCP server publishes these tools:

- `list`
- `list_elements`
- `capture`
- `run`
- `listen`
- `clipboard_get`
- `clipboard_set`
- `permissions`
- `feedback_report`
- `feedback_status`

Those tools are backed by the same typed command models used by the CLI, which keeps validation and result envelopes aligned across both surfaces. MCP `run` calls use the same default focus-restoration and host-local execution lock/queue behavior as CLI `tendril run`; pass `restore_focus`, `no_restore_focus`, `no_lock`, `lock_timeout_ms`, `lock_stale_ms`, or `lock_path` in the tool arguments when an advanced workflow needs to tune focus handling or the queue.

The `permissions` tool reports the host platform's Screen Recording, Accessibility, Microphone, and Camera permission status (granted / denied / unknown / not-required) with remediation guidance, so an MCP client can proactively check setup before a capture or input call fails. It is read-only and performs no capture or input.

The `capture` tool accepts an optional `camera` argument (mutually exclusive with `window`/`display`) to grab a single frame from a video capture device discovered via the `list` tool's `cameras` array. It uses ffmpeg's AVFoundation backend on macOS, V4L2 on Linux, and DirectShow on Windows. macOS capture explicitly requests 30 fps to avoid ffmpeg's incompatible 29.97 fps default on cameras such as the Logitech C930e, with one advertised-rate retry for devices that do not support 30 fps.

The `list`, `list_elements`, `capture`, and `run` tools accept an optional `x11_display` argument (Linux/X11 only) that pins the underlying X11 connection to an explicit display such as `:99`, overriding the server process's ambient `$DISPLAY`. This exists because the MCP server is a single long-lived process whose environment is fixed at spawn: on a headless node that brings up a virtual display (for example Xvfb) *after* the server started, the server's `$DISPLAY` is still unset, so `list` would fail with a `platform_adapter_failure`. Passing `x11_display: ":99"` in the tool arguments lets a client target that display without restarting the server (bd-6abe70). When omitted, the ambient `$DISPLAY` is used, so nothing changes for the fresh-process CLI or for a server launched inside an X session.

On Linux and macOS, the server additionally publishes `self_update_status`, `self_update_check`, and `self_update_run`. These tools come from the shared [`updatable-cli`](https://github.com/harryaskham/updatable-cli) integration, which also implements the `tendril update [run|check|status]` CLI path. Windows builds omit the Unix-oriented updater and return a structured unsupported-platform error for `tendril update`; install Windows builds from source or a separately published package.

The `feedback_*` tools come from the shared [`feedback-cli`](https://github.com/harryaskham/feedback-cli) integration (the sibling of `mcp-cli` / `updatable-cli`). They let an MCP client report a structured feedback event (`feedback_report`) and inspect the resolved reporting destination (`feedback_status`). Tendril also reports its own breakages automatically: every CLI/MCP error that reaches the central error sink is forwarded as a structured `FeedbackEvent` so the owning project can turn it into a bead. The reporting *strategy* is selected from configuration — a `webhook` (e.g. a caco feedback endpoint that files a bead), the local `caco` CLI, a file, or stderr. Feedback is **opt-in**: with nothing configured Tendril stays silent (no extra stderr, no beads). Configure it either with a `[feedback]` block in the Tendril config (see [Configuration](reference/configuration.md#feedback); an explicit config block wins) or, as a fallback, by setting `FEEDBACK_WEBHOOK_URL` (and optionally `FEEDBACK_WEBHOOK_TOKEN_ENV` and `FEEDBACK_PROJECT`) to route breakages to a caco feedback endpoint that creates beads. A shared `FEEDBACK_WEBHOOK_BASE_URL` is also honored: when no full `FEEDBACK_WEBHOOK_URL` is set, Tendril appends its `/tendril` path and routes to `<base>/tendril` (bd-42a4d9).

For the Pi/Cacophony-facing launch contract, environment assumptions, and stable tool argument summary, see the dedicated [Pi and Cacophony MCP integration contract](reference/pi-cacophony-mcp-contract.md).

## Shared transport contract

The `crates/mcp-cli` git submodule (`https://github.com/harryaskham/mcp-cli`) provides the reusable pieces for:

- stable success and error envelopes,
- tool metadata and JSON Schema generation,
- typed tool routing, and
- newline-delimited JSON (NDJSON) stdio transport handling.

### Nix/crane source-grafting footgun

`flake.nix` grafts the pinned `mcp-cli` flake input into `crates/mcp-cli` before crane builds the workspace. Keep that grafted source and Cargo's dependency source identity aligned:

- The root `tendril` package intentionally depends on `mcp-cli` through the same git source/revision that `updatable-cli` expects, matching the other Cacophony CLI projects.
- Do **not** rely on a root `[patch."https://github.com/harryaskham/mcp-cli"]` path override just because a local `cargo build` succeeds. The crane-cleaned/grafted source can present that patch table differently during `cargoArtifacts`, so `updatable-cli` may resolve against a different `mcp-cli` API in the Nix build.
- If you change the `mcp-cli` dependency strategy, validate with the Nix path (`nix build .#tendril .#mcp-cli` or the queued equivalent) before landing, not only with a local Cargo build.


## Minimal wire flow

A raw client should:

1. start `tendril mcp stdio`
2. send `initialize`
3. send `notifications/initialized`
4. request `tools/list`
5. use `tools/call`

Example calls:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list","arguments":{}}}
```

## Smoke coverage

The repository includes both:

- semantic parity coverage between CLI JSON and MCP structured content, and
- an external-client smoke probe that launches Tendril as a subprocess and drives `initialize -> tools/list -> tools/call`.

Manual smoke example:

```bash
./scripts/mcp-stdio-smoke.sh -- nix run .#tendril -- mcp stdio
```

## Relationship to the docs site

The published documentation site is structured so MCP usage lives alongside the CLI guides instead of in a separate site. That reflects the actual product model: one command surface, two transports.
