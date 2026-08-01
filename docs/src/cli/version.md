# `tendril version`

`tendril version` prints the version of the running Tendril binary. It is the command-form equivalent of Clap's built-in `tendril --version` flag and never mutates a source checkout.

```bash
tendril version
# tendril 0.0.4

# Machine-readable form
tendril --json version
```

JSON output contains `name` and `version` fields under the normal Tendril success envelope.

Release version bumps are repository-maintainer operations, not an installed CLI feature. Maintainers update `[workspace.package].version` and `Cargo.lock` in a reviewed source change, then push the matching `vX.Y.Z` tag. The tag-only release workflow verifies that the tag and manifest version match before building any assets.
