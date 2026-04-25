# CLI guide

Tendril keeps its CLI intentionally small and agent-oriented.

## Command map

| Command | Purpose | JSON support | MCP parity |
| --- | --- | --- | --- |
| `tendril list` | Discover windows and displays | Yes | Yes |
| `tendril capture` | Capture a screenshot for a selected display or window | Yes | Yes |
| `tendril run` | Execute text or input sequences against a selected target | Yes | Yes |
| `tendril listen` | Capture WAV audio (PipeWire/PulseAudio on Linux, CoreAudio on macOS) and report capability/permission state | Yes | Yes |
| `tendril alias` | Emit shell wrappers for repeated targeting | Yes | Not yet |
| `tendril mcp stdio` | Serve the initial MCP tool surface over stdio | N/A | N/A |

## Global flags

The root CLI currently shares these global flags across command execution:

- `--json` for stable machine-readable envelopes,
- `--window <id>` to scope target-aware commands to a window, and
- `--display <id>` to scope target-aware commands to a display.

Commands that act on a target require exactly one of `--window` or `--display`.

## `run` execution-lock flags

`tendril run` is serialized by a host-local lock/queue by default. The most common controls are:

- `--no-lock` to opt out for advanced workflows,
- `--lock-timeout-ms <ms>` to bound queue waiting,
- `--lock-stale-ms <ms>` to tune stale heartbeat reaping, and
- `--lock-path <path>` to use a custom lock root.

The same controls are available on the MCP `run` tool. See [Execution lock and queue](../reference/execution-lock.md).

## Recommended flow

```bash
tendril list --json
tendril --window <id> capture --json
tendril --window <id> run 'send("hello")'
```

The command-specific pages below document the current shape of each surface and note where the implementation is intentionally probe-first or scaffolded for future work.
