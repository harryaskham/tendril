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
- `self_update_status`
- `self_update_check`
- `self_update_run`
- `feedback_report`
- `feedback_status`

Those tools are backed by the same typed command models used by the CLI, which keeps validation and result envelopes aligned across both surfaces. MCP `run` calls use the same default focus-restoration and host-local execution lock/queue behavior as CLI `tendril run`; pass `restore_focus`, `no_restore_focus`, `no_lock`, `lock_timeout_ms`, `lock_stale_ms`, or `lock_path` in the tool arguments when an advanced workflow needs to tune focus handling or the queue.

The `self_update_*` tools come from the shared [`updatable-cli`](https://github.com/harryaskham/updatable-cli) integration, matching the ring-mods reference pattern. They let an MCP client inspect the current install path, check GitHub releases, and stage/promote a newer Tendril binary without adding Tendril-specific update protocol code to the client.

The `feedback_*` tools come from the shared [`feedback-cli`](https://github.com/harryaskham/feedback-cli) integration (the sibling of `mcp-cli` / `updatable-cli`). They let an MCP client report a structured feedback event (`feedback_report`) and inspect the resolved reporting destination (`feedback_status`). Tendril also reports its own breakages automatically: every CLI/MCP error that reaches the central error sink is forwarded as a structured `FeedbackEvent` so the owning project can turn it into a bead. The reporting *strategy* is selected from configuration — a `webhook` (e.g. a caco feedback endpoint that files a bead), the local `caco` CLI, a file, or stderr. Feedback is **opt-in**: with nothing configured Tendril stays silent (no extra stderr, no beads). Set `FEEDBACK_WEBHOOK_URL` (and optionally `FEEDBACK_WEBHOOK_TOKEN_ENV` and `FEEDBACK_PROJECT`) to route breakages to a caco feedback endpoint that creates beads.

For the Pi/Cacophony-facing launch contract, environment assumptions, and stable tool argument summary, see the dedicated [Pi and Cacophony MCP integration contract](reference/pi-cacophony-mcp-contract.md).

## Shared transport contract

The `crates/mcp-cli` git submodule (`https://github.com/harryaskham/mcp-cli`) provides the reusable pieces for:

- stable success and error envelopes,
- tool metadata and JSON Schema generation,
- typed tool routing, and
- newline-delimited JSON (NDJSON) stdio transport handling.

### Nix/crane source-grafting footgun

`flake.nix` grafts the pinned `mcp-cli` flake input into `crates/mcp-cli` before crane builds the workspace. Keep that grafted source and Cargo's dependency source identity aligned:

- `crates/tendril` intentionally depends on `mcp-cli` through the same git source/revision that `updatable-cli` expects, matching the ring-mods reference pattern.
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
