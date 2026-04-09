# MCP guide

Tendril exposes its initial command surface over stdio with:

```bash
tendril mcp stdio
```

## Initial tool set

The current MCP server publishes three tools:

- `list`
- `capture`
- `run`

Those tools are backed by the same typed command models used by the CLI, which keeps validation and result envelopes aligned across both surfaces.

## Shared transport contract

The in-repo `mcp-cli` crate provides the reusable pieces for:

- stable success and error envelopes,
- tool metadata and JSON Schema generation,
- typed tool routing, and
- framed stdio transport handling.

## Parity expectations

The repository already includes parity coverage asserting that CLI JSON output and MCP structured content match for the current list, capture, and run flows.

## Relationship to the docs site

The published documentation site is structured so MCP usage lives alongside the CLI guides instead of in a separate site. That reflects the actual product model: one command surface, two transports.
