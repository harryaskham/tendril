# `tendril update`

`tendril update` installs a released Tendril binary for the current platform.
It is intended for operators and agents that want a packaged binary without
building the workspace from source. Tendril also wires the shared `updatable-cli`
MCP extension, so long-running MCP clients can call `self_update_status`,
`self_update_check`, and `self_update_run` over `tendril mcp stdio` to inspect or
apply updates dynamically.

## Examples

```bash
# Plan the latest-release install without writing files.
tendril update --dry-run --json

# Install the latest release to ~/.local/bin/tendril.
tendril update

# Install a specific release version.
tendril update --release-version 0.0.1

# Install to a custom directory.
tendril update --install-dir /tmp/tendril-bin --release-version 0.0.1
```

## Behaviour

The updater:

1. detects the current release platform suffix such as `x86_64-linux` or
   `aarch64-darwin`,
2. resolves either the latest GitHub release or `--release-version`,
3. downloads the matching archive and `.sha256` asset,
4. verifies the checksum,
5. extracts the `tendril` binary, and
6. runs `tendril --version` from the install path to verify the result.

The default repository is `harryaskham/tendril`. Use `--repository owner/name`
for forks or release-candidate repositories.

When the GitHub CLI is available, Tendril prefers `gh release download` so
private or authenticated releases work with the operator's existing `gh` setup.
It falls back to direct release asset URLs with `curl` for public assets.

## JSON shape

In JSON mode, successful output includes:

- `repository`
- `version`
- `tag`
- `platform`
- `archive_url`
- `checksum_url`
- `install_path`
- `installed`
- `verified_version`
- `notes`

Use `--dry-run --json` when an automation flow wants to inspect the selected
asset and install path before allowing writes.
