# Tendril documentation

Tendril is a stateless Rust CLI for agent-driven desktop inspection and control across macOS, Linux, and Windows 11. This site is published as a static GitHub Pages site and is organized around the workflows an agent uses in practice:

1. discover a target with `tendril list`,
2. capture the current state with `tendril capture`,
3. act on that target with `tendril run`, and
4. expose the same typed command surface through `tendril mcp stdio`.

## What this site publishes

This docs site intentionally ships three layers of documentation:

| Layer | Purpose | Published path |
| --- | --- | --- |
| Guides | Human-oriented setup, usage, and workflow docs | `/` |
| Reference | Config, JSON envelope, rollout, and release/publishing details | `/reference/` |
| Rust API docs | Generated `cargo doc` output for the workspace crates | `/api/` |

That split keeps operator-facing guidance separate from generated Rust API material while still publishing everything as one static site.

## Current surface area

The repository currently exposes these user-facing entry points:

| Surface | Status |
| --- | --- |
| `tendril list` | Implemented in CLI and MCP |
| `tendril capture` | Implemented in CLI and MCP |
| `tendril run` | Implemented in CLI and MCP |
| `tendril listen` | Implemented as a probe-first CLI command |
| `tendril alias` | Implemented as a shell-helper CLI command |
| `tendril mcp stdio` | Implements the initial list/capture/run MCP tool set |

## Repository layout

The docs source lives under `docs/`:

```text
docs/
├── book.toml
├── src/
│   ├── index.md
│   ├── quickstart.md
│   ├── cli/
│   └── reference/
└── theme/
    └── nord.css
```

The generated site is built into `target/book/`, and the Pages workflow uploads that directory after copying workspace Rust API docs into `target/book/api/`.

## Read this next

- [Quickstart](quickstart.md)
- [macOS operator validation](macos-operator-validation.md)
- [Linux Wayland operator validation](linux-wayland-operator-validation.md)
- [CLI guide](cli/index.md)
- [MCP guide](mcp.md)
- [Docs publishing and release flow](reference/publishing.md)
