# Quickstart

## Development environment

Tendril is developed as a Rust workspace with Nix and direnv.

```bash
direnv allow
```

That loads the flake-backed development shell, including Rust tooling plus `mdbook` for documentation authoring.

## Build the documentation site locally

Use the shared build script so local output matches CI and GitHub Pages:

```bash
nix develop --command ./scripts/build-docs.sh
```

This command:

1. builds the mdBook content from `docs/src/`,
2. generates workspace rustdoc with `cargo doc --workspace --no-deps`, and
3. assembles a publishable static site in `target/book/`.

Open `target/book/index.html` in a browser to preview the published site.

## Core agent workflow

The CLI is designed around a small, explicit loop:

```bash
tendril list --json
tendril --window <id> capture --json
tendril --window <id> list-elements --json
tendril --window <id> run 'send("hello")'
```

If you are validating Tendril as an operator on a real desktop instead of setting up the development environment, use the dedicated platform guides for copy-pasteable validation steps:

- [macOS operator validation](macos-operator-validation.md)
- [Linux/X11 operator validation](linux-x11-operator-validation.md)
- [Linux Wayland operator validation](linux-wayland-operator-validation.md)

Remote and host-tunnel variants use the same command surface:

```bash
tendril --remote user@host --json list
tendril --wsl-tunnel --json list
```

For repeated use against one target, generate a shell helper instead of storing hidden runtime state:

```bash
eval "$(tendril --window <id> alias --name desk)"
desk capture --json
desk run 'hold(ctrl),c,release(ctrl)'
```

## Runtime config location

Machine-local defaults are loaded from:

- `$TENDRIL_CONFIG_DIR/config.yaml`, or
- `~/.config/tendril/config.yaml` by default.

See [Configuration](reference/configuration.md) for the current schema.

## Publishing model

Documentation publishing follows the same release policy as the rest of the repository:

- docs are built and deployed by GitHub Actions,
- the Pages workflow runs on version tag pushes and release-aligned version bumps, and
- the published site should reflect the current CLI/MCP/platform support matrix.

See [Docs publishing and release flow](reference/publishing.md) for the exact workflow.
