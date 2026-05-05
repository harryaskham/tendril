# `tendril version`

`tendril version bump` updates Tendril's SemVer metadata and creates the release
bump commit.

## Examples

```bash
tendril version bump patch
tendril version bump minor --json
tendril version bump major
```

## Behaviour

The bump command:

1. discovers the git repository root,
2. requires a clean tracked worktree,
3. reads `[workspace.package].version` from `Cargo.toml`,
4. increments the selected SemVer component,
5. updates the workspace manifest, the in-repo `mcp-cli` manifest, and
   Tendril package entries in `Cargo.lock`,
6. stages those files, and
7. creates a commit named `chore(release): bump tendril to v<version>`.

Patch bumps increment only the patch component. Minor bumps increment the minor
component and reset patch to zero. Major bumps increment the major component and
reset minor and patch to zero.

## JSON shape

JSON output includes:

- `previous_version`
- `new_version`
- `level`
- `updated_files`
- `commit`
- `tag`

The command intentionally creates the commit itself so the release bump stays
atomic and easy to tag or let the version-triggered release workflow publish.
