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
- `flake.nix`: dev shell, packages, checks, docs validation, and reproducible release artifacts
- `.cacophony/config.yaml`: project bootstrap plus queued build/test defaults
- `scripts/pre-merge.sh`: fast local validation hook entrypoint
- `scripts/release-artifacts.sh`: full GitHub-release staging helper for binary and source archives
- `scripts/release-notes.sh`: changelog section extractor for GitHub release notes
- `scripts/release-lib.sh`: SemVer, tag, system, and artifact naming helpers shared by release automation
- `scripts/stage-release-artifacts.sh`: Nix-backed staging helper for releasable binary archives, checksums, and manifest metadata
- `.github/workflows/tag-release.yml`: tag-only GitHub Actions validation and release publishing

## Release automation

Local pre-merge validation remains the primary fast-feedback gate:

```bash
./scripts/pre-merge.sh
```

Tendril uses SemVer. The single source of truth for the release version is
`[workspace.package].version` in `Cargo.toml`, and release tags use the
`v<semver>` form such as `v0.0.1`.

Tagged releases intentionally avoid per-commit remote CI. Pushing a `v*` tag
starts the GitHub Actions release workflow, which:

1. installs Nix on the self-hosted runner,
2. reruns `./scripts/pre-merge.sh` so the remote release gate matches local expectations,
3. builds the `.#tendril` flake package reproducibly,
4. packages a versioned binary tarball plus a source tarball under `dist/`, and
5. publishes a GitHub release using notes extracted from `CHANGELOG.md`.

For release packaging, `nix build .#releaseArtifact` produces the canonical
binary archive, checksum, and `release-manifest.json` using the same workspace
version as Cargo. Canonical binary asset names use the formula
`tendril-<semver>-<nix-system>.tar.gz`, with matching `.sha256` sidecars.
Supported Nix system suffixes are: `x86_64-linux`, `aarch64-linux`,
`aarch64-darwin`, and `x86_64-darwin`.

To prepare artifacts locally before pushing a tag:

```bash
./scripts/stage-release-artifacts.sh v0.0.1
./scripts/release-artifacts.sh v0.0.1
./scripts/release-notes.sh v0.0.1
```

See `docs/release-management.md` for the detailed repository release runbook and
`docs/src/reference/publishing.md` for the published docs-site release reference.

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
