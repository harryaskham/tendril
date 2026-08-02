# Release management

Tendril ships through a tag-only, self-hosted release pipeline whose artifacts match the shared `updatable-cli` contract.

## Version source of truth

- The authoritative SemVer is `[workspace.package].version` in `Cargo.toml`.
- `tendril version` only prints the running binary version; installed Tendril binaries never edit repositories or create release commits.
- Maintainers bump `Cargo.toml` and `Cargo.lock` in a reviewed source change, then push the matching stable tag, for example `v0.0.4`.
- The release workflow refuses a tag that does not exactly match the manifest version.

## Supported release targets

The self-hosted matrix builds binaries for:

- `x86_64-linux` natively on `[self-hosted, nix, x86_64-linux]`
- `aarch64-linux` as a static musl cross-build on `[self-hosted, nix, x86_64-linux]`
- `aarch64-darwin` natively on `[self-hosted, nix, aarch64-darwin]`

Native jobs build inside the repository development shell with Cargo:

```bash
nix develop --command cargo build --release --locked -p tendril --bin tendril
```

No native aarch64 Linux Actions runner is registered. That lane runs stable Cargo's `aarch64-unknown-linux-musl` target inside the tag's dev shell with Nixpkgs' sysroot-aware musl cross compiler, producing a static portable binary without changing the immutable tag's flake outputs. Native targets also use the dev shell. Both paths intentionally differ from `nix build .#tendril`: the Nix package wraps the executable to provide runtime dependencies and is not portable outside the Nix store.

## Canonical assets

For each target, CI publishes:

```text
tendril-<semver>-<target>.tar.gz
tendril-<semver>-<target>.sha256
```

The archive contains `tendril-<semver>-<target>/tendril`, exactly where `updatable-cli::AssetStrategy::TendrilStyle` expects it. Darwin publication fails if `otool -L` still reports any `/nix/store` dependency.

The release is created as a draft, each target uploads its pair, and a final self-hosted job publishes only after all six expected files are present.

## Local staging and smoke

```bash
# Build/package the native host target as a raw Cargo binary.
./scripts/stage-release-artifacts.sh v0.0.4

# Optional source archive + manifest alongside the native binary pair.
./scripts/release-artifacts.sh v0.0.4

# Platform-specific packaged smoke helpers.
./scripts/linux-x11-packaged-smoke.sh v0.0.4
./scripts/macos-packaged-smoke.sh v0.0.4
```

## Update verification

Every native release job extracts the exact uploaded archive and runs:

```bash
tendril version
tendril --version
tendril update status
```

That catches wrong archive roots, accidental Nix wrapper shipment, non-portable Darwin linkage, and drift between the CLI updater and MCP `self_update_*` tools before publication.
