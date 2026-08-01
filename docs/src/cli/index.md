# CLI guide

Tendril keeps its CLI intentionally small and agent-oriented.

## Command map

| Command | Purpose | JSON support | MCP parity |
| --- | --- | --- | --- |
| `tendril list` | Discover windows, displays, and cameras (video capture devices) | Yes | Yes |
| `tendril capture` | Capture a screenshot for a selected display/window, or a single frame from a `--camera` device | Yes | Yes |
| `tendril run` | Execute text or input sequences against a selected target | Yes | Yes |
| `tendril listen` | Capture WAV audio (PipeWire/PulseAudio on Linux, CoreAudio on macOS) and report capability/permission state | Yes | Yes |
| `tendril alias` | Emit shell wrappers for repeated targeting | Yes | Not yet |
| `tendril update [run|check|status]` | Update through the shared `updatable-cli` release flow | Yes (Linux/macOS) | Yes (Unix self-update tools) |
| `tendril version` | Print the running Tendril version | Yes | Not yet |
| `tendril permissions` | Report Screen Recording, Accessibility, Microphone, and Camera permission status with remediation guidance (read-only by default; no capture or input). Add `--request` (macOS, foreground sessions) to programmatically surface the OS prompts and open the matching System Settings panes | Yes | Not yet |
| `tendril mcp stdio` | Serve the typed MCP tool surface over stdio | N/A | N/A |

## Global flags

The root CLI currently shares these global flags across command execution:

- `--json` for stable machine-readable envelopes,
- `--window <id>` to scope target-aware commands to a window, and
- `--display <id>` to scope target-aware commands to a display,
- `--camera <id>` to scope `tendril capture` to a video capture device (AVFoundation on macOS, V4L2 on Linux, or DirectShow on Windows; see `tendril list`),
- `--remote user@host` to proxy the invocation over SSH, and
- `--wsl-tunnel` to proxy from WSL/Linux to a Windows-host `tendril.exe`.

Commands that act on a target require exactly one of `--window` or `--display`; `tendril capture` also accepts `--camera` (mutually exclusive with `--window`/`--display`).

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
tendril --window <id> list-elements --json
tendril --window <id> run 'send("hello")'
```

For remote desktops, use `--remote user@host`. For Windows host control from WSL, use `--wsl-tunnel`; install `tendril.exe` on the Windows PATH or point `TENDRIL_WSL_WINDOWS_BIN` at it.

The command-specific pages below document the current shape of each surface and note where the implementation is intentionally probe-first or scaffolded for future work.
