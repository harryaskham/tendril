# CLI guide

Tendril keeps its CLI intentionally small and agent-oriented.

## Command map

| Command | Purpose | JSON support | MCP parity |
| --- | --- | --- | --- |
| `tendril list` | Discover windows and displays | Yes | Yes |
| `tendril capture` | Capture a screenshot for a selected display or window | Yes | Yes |
| `tendril run` | Execute text or input sequences against a selected target | Yes | Yes |
| `tendril listen` | Probe audio capture capability and permission state | Yes | Not yet |
| `tendril alias` | Emit shell wrappers for repeated targeting | Yes | Not yet |
| `tendril mcp stdio` | Serve the initial MCP tool surface over stdio | N/A | N/A |

## Global flags

The root CLI currently shares these global flags across command execution:

- `--json` for stable machine-readable envelopes,
- `--window <id>` to scope target-aware commands to a window, and
- `--display <id>` to scope target-aware commands to a display.

Commands that act on a target require exactly one of `--window` or `--display`.

## Recommended flow

```bash
tendril list --json
tendril --window <id> capture --json
tendril --window <id> run 'send("hello")'
```

The command-specific pages below document the current shape of each surface and note where the implementation is intentionally probe-first or scaffolded for future work.
