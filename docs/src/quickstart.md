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
tendril --window <id> run 'send("hello")'
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

Documentation publishing follows the same tag-oriented release policy as the rest of the repository:

- docs are built and deployed by GitHub Actions,
- the Pages workflow runs on version tag pushes such as `v0.0.1`, and
- the published site always reflects a release-aligned docs snapshot.

See [Docs publishing and release flow](reference/publishing.md) for the exact workflow.
