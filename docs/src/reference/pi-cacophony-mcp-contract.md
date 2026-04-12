# Pi and Cacophony MCP integration contract

This page documents the Tendril-side contract for Pi and Cacophony consumers that want desktop automation through Tendril's external MCP server.

Pi does **not** expose built-in generic MCP client wiring by itself. Per the Pi docs, external integrations should be packaged as a local extension or a Pi package that registers tools on Pi's side. For Tendril, that bridge should launch Tendril as an MCP stdio subprocess and proxy the stable Tendril tools into Pi/Cacophony.

## Supported launch contract

Tendril's public MCP entrypoint is:

```bash
tendril mcp stdio
```

Equivalent release-facing launch forms are also valid as long as they end in the same argv:

```bash
nix run .#tendril -- mcp stdio
/path/to/tendril mcp stdio
```

### Process expectations

- Tendril serves MCP over the child process **stdin/stdout** pair.
- Clients must send framed JSON-RPC messages with `Content-Length` headers.
- Tendril writes MCP responses to stdout and keeps stderr available for diagnostics.
- Closing stdin cleanly ends the server; no daemon or background service is required.
- Tendril is stateless between calls apart from optional config defaults in `$TENDRIL_CONFIG_DIR/config.yaml`.

For raw protocol framing details, see the [MCP guide](../mcp.md).

## Pi packaging assumptions

For Pi consumers, the bridge layer belongs in a Pi extension or Pi package:

- local extensions can live under `.pi/extensions/` or `~/.pi/agent/extensions/`, and
- shared integrations can be published as Pi packages and loaded through `packages` in `.pi/settings.json`.

Tendril does not prescribe the Pi-side wrapper implementation, but the wrapper should treat `tendril mcp stdio` as the only launch contract it depends on.

## Runtime environment and session assumptions

Tendril expects to run inside an active local desktop session for the current user.

### Common assumptions

- Tendril must run on the same machine as the desktop session it is inspecting.
- The process should inherit the normal GUI-session environment for the operator account.
- `TENDRIL_CONFIG_DIR` is optional; if unset, Tendril falls back to the standard user config path.
- Target ids are workflow-scoped: call `list`, then pass those ids into later `capture` or `run` calls.

### Linux

- Tendril expects a detected graphical session.
- X11 support depends on session variables such as `DISPLAY` and `XDG_SESSION_TYPE=x11`.
- Wayland support depends on the active compositor/session tools available to the logged-in user.
- If no usable session is detected, Tendril returns a structured capability error instead of hanging.

### macOS

- Tendril must run in the logged-in user's WindowServer session.
- `list` and `capture` depend on **Screen Recording** consent.
- `run` depends on **Accessibility** consent.
- `listen --source microphone` depends on **Microphone** consent.

### Windows 11

- Tendril expects the normal interactive desktop session for the current user.
- Desktop/session capability failures are reported as structured errors.

## Stable MCP tool contract

The current stable tool names are:

- `list`
- `capture`
- `run`

Those names are semver-relevant.

### Arguments

| Tool | Required arguments | Optional arguments | Notes |
| --- | --- | --- | --- |
| `list` | none | none | Discovers windows and displays. |
| `capture` | exactly one of `window` or `display` | `max_width`, `max_height`, `format`, `compression` | Uses the same typed request model as CLI capture. |
| `run` | exactly one of `window` or `display`, plus `input_definition` | none | `input_definition` is the same text/DSL field accepted by CLI `run`. |

Representative `tools/call` payloads:

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list","arguments":{}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"capture","arguments":{"window":"window-1","max_width":1440}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"run","arguments":{"window":"window-1","input_definition":"send(\"hello\")"}}}
```

## Schema and semver expectations

Tendril treats MCP tool names, input schemas, and structured result envelopes as versioned public interface.

The authoritative schema source for clients is the `tools/list` response emitted by the running server. In the implementation, those schemas are generated from the same typed Rust request models used by the CLI:

- `list` -> `ListCommand`
- `capture` -> `CaptureRequest`
- `run` -> `RunRequest`

Repository tests assert that those schemas stay aligned with the CLI-facing models, so contract drift is caught as a semver-relevant change.

## Result envelopes

`tools/call` returns `structuredContent` that matches Tendril's CLI `--json` envelope:

- success responses use `{"status":"success","meta":{"command":"..."},"data":...}`
- error responses use `{"status":"error","meta":{"command":"..."},"error":...}`

That means Pi/Cacophony wrappers can share one result parser across CLI JSON and MCP tool execution.

## External-client smoke path

The repository includes a client-oriented smoke probe that launches Tendril as a subprocess, performs:

1. `initialize`
2. `tools/list`
3. `tools/call` for `list`

and verifies that the returned tool metadata and schemas match the published contract.

Manual examples:

```bash
./scripts/mcp-stdio-smoke.sh -- nix run .#tendril -- mcp stdio
./scripts/mcp-stdio-smoke.sh -- /path/to/tendril mcp stdio
```

The automated Rust integration test `crates/tendril/tests/mcp_external_smoke.rs` runs the same smoke path against the built Tendril binary with fixture-backed target data so non-Tendril clients are covered in CI as well.
