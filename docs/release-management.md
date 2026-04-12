# Release management

Tendril ships from a tag-driven release pipeline. This document defines the
version source of truth, canonical artifact names, and the reproducible staging
steps required for the initial `v0.0.1` release.

## SemVer and source of truth

- Tendril uses SemVer.
- The authoritative version lives in `[workspace.package].version` in `Cargo.toml`.
- The initial release target is `0.0.1`, published as the git tag `v0.0.1`.
- Stable CLI flags, JSON schemas, and MCP tool contracts are semver-relevant.

Cargo crate manifests inherit the workspace version and repository metadata so
`cargo`, clap version output, and package metadata remain aligned.

## Canonical release assets

The release pipeline publishes one archive plus one checksum per platform.
Canonical binary archive names follow this formula:

```text
tendril-<semver>-<nix-system>.tar.gz
```

Canonical binary checksum names follow this formula:

```text
tendril-<semver>-<nix-system>.sha256
```

Supported Nix system suffixes for `v0.0.1` are:

- `x86_64-linux`
- `aarch64-linux`
- `aarch64-darwin`
- `x86_64-darwin`

Each binary archive contains a single executable named `tendril`.
The tag-driven GitHub release workflow also publishes a
`tendril-<semver>-source.tar.gz` source archive.

## Nix packaging outputs

The flake exposes two release-relevant packages:

- `.#tendril` — the platform-native Tendril binary package.
- `.#releaseArtifact` — a reproducible output directory containing:
  - the canonical binary `tar.gz` archive,
  - the matching `.sha256` file, and
  - `release-manifest.json` describing the version, tag, system, and asset names.

`releaseArtifact` derives its version from the same workspace manifest used by
Cargo, so Rust package metadata and Nix release assets cannot drift silently.

## Release helpers

Four scripts are checked into `scripts/` for release automation and local dry runs:

- `scripts/release-lib.sh`
  - prints the current SemVer, tag, Nix system suffix, and canonical binary file names.
- `scripts/stage-release-artifacts.sh`
  - validates that the supplied tag matches `Cargo.toml`,
  - runs `nix build .#releaseArtifact`, and
  - copies the binary archive, checksum, and manifest into `dist/release/`.
- `scripts/release-artifacts.sh`
  - assembles the current end-to-end GitHub release payload under `dist/`,
  - using the same SemVer/tag validation plus the matching source archive.
- `scripts/linux-x11-packaged-smoke.sh`
  - extracts the packaged Linux artifact,
  - runs packaged `list` and `capture` inside a real X11 session, and
  - verifies the binary no longer fails on missing `xrandr`, `xprop`, `xwininfo`, `import`, or optional `xdotool` helper dependencies.

Examples:

```bash
./scripts/release-lib.sh version
./scripts/release-lib.sh tag
./scripts/stage-release-artifacts.sh v0.0.1
./scripts/linux-x11-packaged-smoke.sh v0.0.1
```

## Publication flow

1. Update `CHANGELOG.md` and ensure `Cargo.toml` contains the release SemVer.
2. Push the matching git tag, for example `v0.0.1`.
3. For a local dry run of the binary release package, stage artifacts:
   ```bash
   ./scripts/stage-release-artifacts.sh v0.0.1
   ```
4. For the full GitHub release payload used by the current tag workflow, build:
   ```bash
   ./scripts/release-artifacts.sh v0.0.1
   ./scripts/release-notes.sh v0.0.1 > dist/release-notes.md
   ```
5. Publish the staged files with GitHub Releases:
   ```bash
   gh release create v0.0.1 --verify-tag --title v0.0.1 --notes-file dist/release-notes.md || true
   gh release upload v0.0.1 dist/*.tar.gz dist/*.sha256 dist/release-manifest.json --clobber
   ```

In CI, the tag-push workflow should call the same release scripts so the
published assets match local dry runs exactly.
