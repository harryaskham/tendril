# bd-39d596 — expose `tendril permissions` as an MCP tool

Follow-up to bd-85da62. Registers the `permissions` command as an MCP tool so
MCP-driving agents (tendril's primary consumers) can proactively check Screen
Recording / Accessibility / Microphone status before a capture or input call
fails, rather than only learning of missing consent via an error envelope.

## Changes
- `commands/mod.rs`: `permissions` typed MCP tool in build_tool_router() (after
  clipboard_set), reusing PermissionsCommand + PermissionsReport via
  context.adapter().permissions(). Updated the in-crate tool-list fixture.
  `#[allow(clippy::too_many_lines)]` on build_tool_router (flat registration list).
- `tests/mcp_external_smoke.rs`: added "permissions" to the asserted tool list.
- `docs/src/mcp.md`: tool bullet + description paragraph.

## Validation
- `cargo clippy -p tendril --all-targets -- -D warnings` clean.
- `cargo test -p tendril --lib` = 328 passed; `mcp_external_smoke` passed.
