# `tendril update`

`tendril update` uses the shared [`updatable-cli`](https://github.com/harryaskham/updatable-cli) implementation used by other Cacophony Rust CLIs. Tendril owns only CLI/JSON presentation; `updatable-cli` performs release discovery, target selection, download, SHA-256 verification, extraction, staging, and promotion.

## Examples

```bash
# Download, verify, and promote the latest release (default action).
tendril update
# Equivalent explicit spelling:
tendril update run

# Read-only release check.
tendril update check
tendril --json update check

# Read-only local install/staging status.
tendril update status

# Optional release source/install-directory overrides.
tendril update check --repository owner/fork
tendril update status --install-dir /opt/tendril/bin
```

## Install contract

The shared updater expects `AssetStrategy::TendrilStyle` assets:

```text
tendril-<version>-<target>.tar.gz
tendril-<version>-<target>.sha256
```

The archive must contain:

```text
tendril-<version>-<target>/tendril
```

Supported published targets are `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin`. By default the updater installs to `$HOME/.local/bin/tendril`, using `$HOME/.local/bin/tendril_next` as the staging path.

The same `UpdaterConfig` powers the MCP tools `self_update_status`, `self_update_check`, and `self_update_run`, so CLI and MCP updates cannot drift into separate implementations.

Windows builds currently report a structured `update_unsupported_platform` response because the shared updater's executable-bit and re-exec contract is Unix-oriented. Install Windows builds from source or a separately published Windows package.

## Output

- `update status` reports current version, install directory, installed path, and staged-next state.
- `update check` reports the latest tag/version, release URL, asset names, and whether it is newer.
- `update` / `update run` reports current/latest versions, staging/promotion state, paths, and any no-update note.

All actions support Tendril's global `--json` envelope.
