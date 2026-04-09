# `tendril list`

Use `tendril list` to discover available display and window targets.

## Example

```bash
tendril list --json
```

## What it returns

The structured response includes:

- adapter metadata,
- current permission state, and
- a list of targets with identifiers, bounds, scale factors, titles, app names, and capability flags.

## Why it matters

`list` is the first step for both CLI and MCP workflows because later commands use the returned target identifiers.

## Notes

- Window and display targets are included by default.
- Audio sources are represented in the model but are not yet included in the default discovery output.
- Permission issues are returned as structured errors rather than opaque text when `--json` is enabled.
