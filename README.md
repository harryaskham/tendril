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
