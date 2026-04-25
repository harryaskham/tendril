# bd-497145 validation summary

Updated MCP external smoke/contract expectations for the current `run` schema, including focus restoration and execution-lock options.

## Focused validation

- `cargo fmt --check` — passed
- `cargo test -p tendril --test mcp_parity` — passed
- `cargo test -p tendril --test mcp_external_smoke` — passed
- `nix run . -- list --json` — passed; output saved to `bd-497145-nix-run-list.json` with stderr in `bd-497145-nix-run-list.stderr`
- `./scripts/mcp-stdio-smoke.sh -- nix run .#tendril -- mcp stdio` — passed; stdout/stderr saved to `bd-497145-mcp-stdio-smoke.out` and `bd-497145-mcp-stdio-smoke.err`

## Notes

- `nix run . -- list --json` returned `status=success`, `meta.command=list`, and 7 discovered targets on this host.
- No Tendril capture artifact was produced for this bead; only list/MCP smoke artifacts were required for the failing path.
