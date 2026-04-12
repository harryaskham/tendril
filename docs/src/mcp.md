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

For the Pi/Cacophony-facing launch contract, environment assumptions, and stable tool argument summary, see the dedicated [Pi and Cacophony MCP integration contract](reference/pi-cacophony-mcp-contract.md).

## Shared transport contract

The in-repo `mcp-cli` crate provides the reusable pieces for:

- stable success and error envelopes,
- tool metadata and JSON Schema generation,
- typed tool routing, and
- framed stdio transport handling.

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
