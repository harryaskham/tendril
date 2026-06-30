# Session summary — Try sandbox=true to unblock tendril CI on azure-ephemeral

## Goal

Tendril CI (`nix flake check` on azure-ephemeral) was blocked by the runner's
non-sandboxed Nix build purity guard (`/homeless-shelter exists`). Attempt a
clean, in-lane self-unblock: force `--option sandbox true` so builds run pure and
bypass that guard — which is also the correct setting for Nix CI.

## Bead(s)

- `bd-acf75f` — tendril CI blocked: azure-ephemeral runners fail nix builds (/homeless-shelter, non-sandboxed)
- (context: bd-08c2bb added CI; bd-2d8a8a fixed fmt; the runner image fix is mono-owned)

## Before state

- Failing tests: CI red — `error: home directory "/homeless-shelter" exists; please remove it to assure purity of builds without sandboxing` on the build/test/clippy derivations (fmt passes).
- ci.yml ran `nix flake check --print-build-logs` (no sandbox override).

## After state

- ci.yml runs `nix flake check --print-build-logs --option sandbox true`.
- Outcome pending the CI run on this land: either CI goes green (runner supports sandboxing — tendril unblocked), or it errors with sandbox-unsupported (confirming the fix must move to the mono runner image).

## Diff summary

- Code commit: bd-acf75f ci.yml one-line change. Final squash SHA from the reintegration receipt.
- Files touched: .github/workflows/ci.yml (1 line).
- Tests: +0 / -0.
- Behavioural delta: CI Nix builds now request sandboxing.

## Operator-takeaway

Bounded self-unblock experiment for the azure-ephemeral /homeless-shelter blocker.
If sandbox=true works, every nix-CI project could adopt the same one-liner as an
interim fix while mono fixes the runner image properly; if not, it's definitively
a runner-image fix (mono).
