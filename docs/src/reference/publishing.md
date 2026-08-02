# Docs publishing and release flow

Tendril publishes documentation as a static GitHub Pages site and publishes raw Cargo binaries for the shared `updatable-cli` updater.

## Documentation

| Path | Purpose |
| --- | --- |
| `docs/book.toml` | mdBook configuration |
| `docs/src/` | Human-authored guide and reference content |
| `scripts/build-docs.sh` | Shared local/CI site assembly entry point |
| `target/book/` | Final Pages artifact |
| `.github/workflows/pages.yaml` | Tag-triggered Pages deployment |

`mdbook build docs` renders the guide, while `cargo doc --workspace --no-deps` generates API docs copied into `target/book/api/`.

## Tag-only binary releases

`.github/workflows/tag-release.yml` runs only for stable `vX.Y.Z` tag pushes. It first verifies that the tag exactly matches `[workspace.package].version` in `Cargo.toml`, then uses self-hosted runners for:

- `x86_64-linux`
- `aarch64-linux`
- `aarch64-darwin`

Native targets build the raw binary with:

```bash
nix develop --command cargo build --release --locked -p tendril --bin tendril
```

No native aarch64 Linux Actions runner is registered, so that lane runs the `aarch64-unknown-linux-musl` target with Nixpkgs' host-runnable cross Cargo, rustc, and musl compiler wrappers on a self-hosted x86_64 Nix runner. Avoiding rustup downloads and the heavyweight desktop dev shell prevents target-component/network failures and camera/browser packages from exhausting the build volume; the resulting static binary remains portable without requiring new flake outputs in the immutable tag. The workflow deliberately does **not** copy `nix build .#tendril` into release archives. The Nix package is wrapped for runtime dependencies and stores its real executable as `.tendril-wrapped`; that wrapper is valid in the Nix store but is not a portable `$HOME/.local/bin` update.

On Darwin, release staging rewrites the Nix-provided libiconv reference to the ABI-compatible system `/usr/lib/libiconv.2.dylib` and refuses publication if any `/nix/store` linkage remains.

## Updatable-cli asset contract

Each self-hosted build publishes the exact `AssetStrategy::TendrilStyle` pair:

```text
tendril-<semver>-<target>.tar.gz
tendril-<semver>-<target>.sha256
```

The tarball contains:

```text
tendril-<semver>-<target>/tendril
```

Before upload, CI extracts that archive and runs `tendril version`, `tendril --version`, and `tendril update status` against the packaged executable. The release remains a draft until all three target archive/checksum pairs exist.

## Local preview

```bash
nix develop --command ./scripts/build-docs.sh
./scripts/stage-release-artifacts.sh v0.0.4
```

The local staging script uses the same raw-Cargo archive layout and portability checks as CI.
