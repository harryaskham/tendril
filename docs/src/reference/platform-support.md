# Platform support and rollout status

Tendril is designed around a platform adapter boundary so the CLI and MCP surface stay stable while platform-specific capture and input implementations evolve.

## Current shape in this repository

| Capability | CLI surface | Current status |
| --- | --- | --- |
| Target discovery | `list` | Implemented |
| Screenshot capture | `capture` | Implemented |
| Input execution | `run` | Implemented |
| MCP stdio | `mcp stdio` | Implemented for list/capture/run |
| Audio probing | `listen` | Probe-first implementation |
| Shell helper generation | `alias` | Implemented |

## Notes

- Discovery currently focuses on windows and displays.
- Audio source modeling exists, but end-to-end audio artifact capture is not yet shipped.
- The docs site intentionally documents both fully implemented features and probe-first surfaces so the published contract matches the repository state.
- For a source-backed inventory of runtime subprocess/tool dependencies and their current self-containment classification, see [Runtime dependency audit](runtime-dependencies.md).
