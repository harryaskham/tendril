# Docs publishing and release flow

This repository publishes documentation as a static GitHub Pages site built from the checked-in `docs/` source tree.

## Source and output paths

| Path | Purpose |
| --- | --- |
| `docs/book.toml` | mdBook configuration |
| `docs/src/` | Human-authored guide and reference content |
| `docs/theme/nord.css` | Dark-only Nord theme overrides |
| `scripts/build-docs.sh` | Shared local/CI site assembly entry point |
| `target/book/` | Final Pages artifact uploaded by CI |
| `target/book/api/` | Generated rustdoc copied into the published site |
| `.github/workflows/pages.yaml` | Tag-triggered Pages deployment workflow |

## Build process

The Pages artifact is assembled in two steps:

1. `mdbook build docs` renders the guide and reference site into `target/book/`.
2. `cargo doc --workspace --no-deps` generates Rust API docs, which are then copied into `target/book/api/`.

That produces one static site containing both narrative docs and generated Rust API docs.

## Deployment policy

The workflow is intentionally aligned with the repository's tag-oriented release policy:

- GitHub Actions runs on version tags such as `v0.0.1`
- the release version source of truth is `[workspace.package].version` in `Cargo.toml`
- the build runs inside the Nix development environment
- `nix build .#releaseArtifact` produces the canonical binary release archive, checksum, and manifest for the current Nix system
- the resulting `target/book/` directory is uploaded to GitHub Pages

This keeps published docs tied to release snapshots instead of publishing every branch push.

## Release artifacts

Binary release assets use the canonical naming pattern
`tendril-<semver>-<nix-system>.tar.gz` with a matching `.sha256` file.
The current tag workflow also publishes `tendril-<semver>-source.tar.gz` and a
`release-manifest.json` file that records the version, tag, system, and artifact list.

## Local preview

```bash
nix develop --command ./scripts/build-docs.sh
```

Then open `target/book/index.html`.
