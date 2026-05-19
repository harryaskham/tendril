# Session summary — Crane patch footgun docs

## Goal

Document the known crane/Cargo patch-table source-grafting trap so future Tendril agents do not trust a local Cargo-only build when changing the `mcp-cli` dependency strategy.

## Bead(s)

- `bd-30a72c` — Document or fix crane Cargo patch-table rewrite footgun

## Before state

- Failing tests: none known.
- Relevant metrics: The earlier failure mode was only captured in a reflection bead; in-tree docs did not explain why Tendril keeps the `mcp-cli` git dependency source identity aligned with `updatable-cli` instead of relying on a root `[patch]` path override.
- Context: `flake.nix` grafts the pinned `mcp-cli` input into `crates/mcp-cli` for crane builds, but crane-cleaned sources can present patch tables differently during `cargoArtifacts`.

## After state

- Failing tests: none observed in focused validation.
- Relevant metrics: `nix flake check --no-build` evaluated successfully; queued build `bj-ae4d5971` succeeded for `nix build .#checks.aarch64-darwin.fmt .#checks.aarch64-darwin.docs --no-link`.
- Context: `docs/src/mcp.md` now has a dedicated Nix/crane source-grafting footgun section, and `flake.nix` has an inline warning next to `graftMcpCli`.

## Diff summary

- Code/content commits: `f99e95b` (`bd-30a72c: document crane patch footgun`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `flake.nix`, `docs/src/mcp.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: No runtime behaviour changed; maintainers now have explicit guidance to validate `mcp-cli` dependency-strategy changes through Nix/crane rather than relying on local Cargo success.

## Operator-takeaway

This closes a dev-experience paper cut: the next worker who considers a root `mcp-cli` path patch will see both the flake comment and docs warning before recreating the Nix-only failure.
