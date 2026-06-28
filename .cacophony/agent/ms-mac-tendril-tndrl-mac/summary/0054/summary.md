# Session summary — move tendril x86 CI to azure-ephemeral runners

## Goal

Operator (Harry) directive: azure-ephemeral GitHub runners (x86-capable) are being brought
up; one agent per project moves CI onto them via `runs-on: [self-hosted, azure-ephemeral]`.
This is the tendril project's change.

## Bead(s)

- `bd-e74ff4` — CI: move tendril x86 Linux jobs to [self-hosted, azure-ephemeral] runners

## Before state

- `.github/workflows/tag-release.yml`: 3 jobs on `[self-hosted, linux]`, 1 on `[self-hosted, macos]`, 1 on `windows-latest`.
- `.github/workflows/pages.yaml`: 2 jobs on `[self-hosted, linux]`.

## After state

- The 5 x86 Linux jobs now use `runs-on: [self-hosted, azure-ephemeral]`.
- `build-macos` (`[self-hosted, macos]`) and `build-windows` (`windows-latest`) deliberately
  LEFT unchanged — azure-ephemeral is x86 Linux only and cannot build those platform artifacts.
- Both workflow files validate (yq); diff is value-only swaps; `git diff --check` clean.

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt.
- Files: `.github/workflows/tag-release.yml` (3 runs-on), `.github/workflows/pages.yaml` (2 runs-on).
- Tests: none (CI config value swap; no source change).
- Behavioural delta: tendril x86 Linux CI jobs target the new azure-ephemeral self-hosted runner pool.

## Operator-takeaway

Tendril x86 CI now points at the azure-ephemeral runners. macOS + Windows release jobs stayed on
their platform runners by design (azure-ephemeral is x86 only) — if/when arm64/macOS azure-ephemeral
runners exist, those can move too.
