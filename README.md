# tendril

Rust workspace scaffold for the Tendril CLI and in-repo `mcp-cli` support crate.

## Quickstart

```bash
direnv allow
```

The repository enters via `use flake`, exposing a development shell with Rust,
clippy, rustfmt, rust-analyzer, and Nix formatting tools.

## Workspace layout

- `crates/tendril`: binary crate scaffold for the Tendril CLI
- `crates/mcp-cli`: reusable structured JSON and MCP façade scaffold
- `flake.nix`: dev shell, packages, and checks
- `.cacophony/config.yaml`: project bootstrap plus queued build/test defaults
- `scripts/pre-merge.sh`: fast local validation hook entrypoint

## Audio capture status

For v0.0.1, `tendril listen` ships a probe-first slice:

- it accepts explicit `--source`, `--duration-ms`, and `--format` settings,
- it returns machine-readable capability and permission diagnostics for loopback/system and microphone paths where the current adapter can probe them,
- it distinguishes unsupported capability/permission failures from transient platform adapter failures, and
- it explicitly reports that audio artifact emission is not implemented yet.

Documented gap for v0.0.1: explicit `device:<id>` binding is accepted by the command surface so callers can express intent, but it returns a structured unsupported-capability result until adapter-specific device enumeration/binding lands.
