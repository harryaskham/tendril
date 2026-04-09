# tendril

Rust workspace scaffold for the Tendril CLI and in-repo `mcp-cli` support crate.

## Quickstart

```bash
direnv allow
```

The repository enters via `use flake`, exposing a development shell with Rust,
clippy, rustfmt, rust-analyzer, and Nix formatting tools.

## Workspace layout

- `crates/tendril`: binary crate scaffold for the Tendril CLI
- `crates/mcp-cli`: reusable structured JSON and MCP façade scaffold
- `docs/`: mdBook-based GitHub Pages source for guides, reference docs, and published rustdoc links
- `flake.nix`: dev shell, packages, and checks
- `.cacophony/config.yaml`: project bootstrap plus queued build/test defaults
- `scripts/pre-merge.sh`: fast local validation hook entrypoint
- `.github/workflows/tag-release.yml`: tag-only GitHub Actions validation and release publishing

## Release automation

Local pre-merge validation remains the primary fast-feedback gate:

```bash
./scripts/pre-merge.sh
```

Tagged releases intentionally avoid per-commit remote CI. Pushing a `v*` tag
starts the GitHub Actions release workflow, which:

1. installs Nix on the self-hosted runner,
2. reruns `./scripts/pre-merge.sh` so the remote release gate matches local expectations,
3. builds the `.#tendril` flake package reproducibly,
4. packages a versioned binary tarball plus a source tarball under `dist/`, and
5. publishes a GitHub release using notes extracted from `CHANGELOG.md`.

To prepare artifacts locally before pushing a tag:

```bash
./scripts/release-artifacts.sh v0.0.1
./scripts/release-notes.sh v0.0.1
```

## Documentation site

The repository publishes a static docs site built from `docs/`.

- local build: `nix develop --command ./scripts/build-docs.sh`
- mdBook source: `docs/src/`
- published Pages artifact: `target/book/`
- generated Rust API docs: `target/book/api/`
- deployment workflow: `.github/workflows/pages.yaml`

The Pages workflow is intentionally tag-triggered so published docs track release snapshots.

## Audio capture status

For v0.0.1, `tendril listen` ships a probe-first slice:

- it accepts explicit `--source`, `--duration-ms`, and `--format` settings,
- it returns machine-readable capability and permission diagnostics for loopback/system and microphone paths where the current adapter can probe them,
- it distinguishes unsupported capability/permission failures from transient platform adapter failures, and
- it explicitly reports that audio artifact emission is not implemented yet.

Documented gap for v0.0.1: explicit `device:<id>` binding is accepted by the command surface so callers can express intent, but it returns a structured unsupported-capability result until adapter-specific device enumeration/binding lands.
