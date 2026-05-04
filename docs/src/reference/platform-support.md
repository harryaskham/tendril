# Platform support and rollout status

Tendril is designed around a platform adapter boundary so the CLI and MCP surface stay stable while platform-specific capture and input implementations evolve.

## Current shape in this repository

| Capability | CLI surface | Current status |
| --- | --- | --- |
| Target discovery | `list` | Implemented |
| Screenshot capture | `capture` | Implemented |
| Input execution | `run` | Implemented |
| Accessibility element discovery | `list-elements` | Implemented with macOS AX, Linux/X11 surface tree, and Linux/Wayland AT-SPI backends |
| MCP stdio | `mcp stdio` | Implemented for list/capture/run/list-elements |
| Audio capture | `listen` | WAV capture via `pw-record`/`parecord` (Linux) and `afrecord` (macOS); probe-only fallback elsewhere |
| Shell helper generation | `alias` | Implemented |

## Notes

- Discovery currently focuses on windows and displays.
- `listen` writes WAV bytes to a temp file (or `--output <path>`) on Linux and macOS; the JSON envelope reports the on-disk path under `execution.artifact`. Windows and unrecognized backends still return `status = "probe_only"`.
- Linux/Wayland is a documented backend matrix rather than one generic path: Hyprland discovery uses `hyprctl`, sway uses `swaymsg`, wlroots display fallback uses `wlr-randr`, capture prefers xdg-desktop-portal with `grim` retained only as a compatibility fallback, and `list-elements` uses AT-SPI when applications publish accessibility metadata.
- The dedicated [Linux Wayland operator validation](../linux-wayland-operator-validation.md) guide covers those supported matrices explicitly.
- The docs site intentionally documents both fully implemented features and probe-first surfaces so the published contract matches the repository state.
- For a source-backed inventory of runtime subprocess/tool dependencies and their current self-containment classification, see [Runtime dependency audit](runtime-dependencies.md).
