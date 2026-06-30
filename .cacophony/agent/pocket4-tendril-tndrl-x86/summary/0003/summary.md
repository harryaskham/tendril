# Session summary — Add PR/push CI workflow to keep tendril main green

## Goal

Per Harry's 2026-06-30 directive, main-green-keeping should move off the local
reintegration gate and into a GitHub Action so dev reints can use PR mode and CI
keeps main green. Tendril had no push/PR CI workflow at all — only release
(`tag-release.yml`) and docs (`pages.yaml`). This session adds the missing
GitHub Actions CI check that runs the same Nix checks the local gate runs, on the
new azure-ephemeral self-hosted runner pool, so a green status check exists for
branch protection to require.

## Bead(s)

- `bd-08c2bb` — Add PR/push CI workflow (nix flake check on azure-ephemeral) so CI keeps tendril main green for PR-backed reints

## Before state

- Failing tests: none touched.
- `.github/workflows/` contained only `pages.yaml` (docs deploy) and `tag-release.yml` (release artifacts). No CI ran on pull_request or push to main.
- Tendril main was kept green solely by the caco reint gate / `scripts/pre-merge.sh` (`nix flake check`).
- The azure-ephemeral runner pool went live this session (Nix+flakes preinstalled, no bare toolchains); tendril is already an enrolled repo.

## After state

- Failing tests: none.
- New `.github/workflows/ci.yml`: job `checks` on `runs-on: [self-hosted, azure-ephemeral]`, triggered on `pull_request` + `push` to `main`, running `nix flake check --print-build-logs` (fmt/clippy/tests/docs) after `nix --version`. Checkout uses `submodules: recursive` (mcp-cli) + `fetch-depth: 0`.
- Validated: YAML parses (yq) — triggers, steps, runner tag, and final `nix flake check` command all confirmed. Flake exposes the referenced `checks.{fmt,clippy,tests,docs}` attrs.
- Everything runs through Nix, so it works on azure-ephemeral with no bare toolchains.

## Diff summary

- Code/content commit: `befa7d0` (bd-08c2bb). Final landed squash SHA from the reintegration receipt.
- Files touched: `.github/workflows/ci.yml` (new, +43 lines).
- Tests: +0 / -0 (CI config; the workflow runs the existing flake checks).
- Behavioural delta: PRs and pushes to main now get a Nix-checks CI run; provides the green status check that branch protection can later require.

## Operator-takeaway

Tendril now has a real CI green-keeping workflow (`nix flake check` on azure-ephemeral).
The remaining 3-leg auto-merge cutover — enabling `allow_auto_merge` + `allow_update_branch`
on the repo and adding branch protection requiring this CI check (strict=true) — is
operator-gated and sequence-sensitive (must not enable auto-merge before a green required
check exists), and GitHub merge queue is unavailable (needs an org). Those steps are being
surfaced to Harry as a caco choice once this workflow lands and runs green once.
