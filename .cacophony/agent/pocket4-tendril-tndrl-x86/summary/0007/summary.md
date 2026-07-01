# Session summary — Route tendril nix CI/release/pages workflows off the broken azure-ephemeral pool onto tendril's own NixOS runners

## Goal

tendril's GitHub Actions CI (and the pages + tag-release workflows) could not go green because they ran their Nix jobs on the shared org-wide `azure-ephemeral` self-hosted pool, which runs Nix non-sandboxed with a pre-existing `/homeless-shelter` HOME placeholder. Nix refuses to build impurely in that state, so every `nix flake check` / `nix build` / `nix develop` job failed the purity guard. The prior session concluded there was "no tendril-side fix" (the fix was assumed to require the mono-owned runner image). This session overturned that: tendril has its own dedicated NixOS runners, and re-pointing the workflows at them is a real, in-scope fix — exactly what mono's infra guidance ("route heavy nix builds to self-hostable labels — projects' responsibility") sanctions.

## Bead(s)

- `bd-acf75f` — tendril CI blocked: azure-ephemeral runners fail nix builds (/homeless-shelter, non-sandboxed) [P1, bug]

## Before state

- CI (`ci.yml`), `pages.yaml`, and `tag-release.yml` all pinned their Linux/nix jobs to `runs-on: [self-hosted, azure-ephemeral]`.
- Latest CI run (28414531958, push to main) FAILED with: `error: home directory "/homeless-shelter" exists; please remove it to assure purity of builds without sandboxing`. A prior tendril-side mitigation (`--option sandbox true`) was ignored by that pool's daemon (untrusted runner user / locked setting).
- tendril CI red on main since 2026-06-30; bead parked as blocked-on-mono.

## After state

- All tendril Nix jobs re-pointed to tendril's own dedicated NixOS runners via `runs-on: [self-hosted, nix, x86_64-linux]`:
  - `ci.yml`: `checks`
  - `pages.yaml`: `build`, `deploy`
  - `tag-release.yml`: `verify`, `build-linux`, `publish`
  - (`tag-release.yml` `build-macos` = `[self-hosted, macos]` and `build-windows` = `windows-latest` left untouched.)
- Validated: the label set `[self-hosted, nix, x86_64-linux]` matches all 5 online tendril Linux NixOS runners (tendril-aurora/beelink/sonance/ms-dev/ms-dev-2) and does NOT match the macOS runner (tendril-ms-mac, aarch64-darwin).
- Hypothesis validated on a peer NixOS box (pocket4): `sandbox = true` is the default and there is no `/homeless-shelter`, so `nix flake check` builds purely there. `azure-ephemeral` has 0 runners registered to tendril (it is a separate org-wide pool).
- `--option sandbox true` retained in ci.yml as belt-and-suspenders (now genuinely effective on properly-configured NixOS runners).

## Diff summary

- Code/content commits: pending final squash SHA from the reintegration receipt.
- Files touched: `.github/workflows/ci.yml` (runs-on + explanatory comments), `.github/workflows/pages.yaml` (2 jobs), `.github/workflows/tag-release.yml` (3 Linux jobs).
- Tests: none added (CI-config change; validated via `yq` YAML parse + runner-label match against the live GitHub runner registry).
- Behavioural delta: tendril's Nix CI/pages/release jobs now execute on tendril's own sandboxed NixOS runners instead of the broken azure-ephemeral pool, unblocking main-green CI and the release/pages pipelines.

## Operator-takeaway

The azure-ephemeral pool is broken for Nix (non-sandboxed + stale /homeless-shelter) and mono considers heavy-nix routing a per-project responsibility. tendril already has five dedicated online NixOS runners, so the correct home for all tendril Nix jobs is `[self-hosted, nix, x86_64-linux]`, not the shared azure pool. This supersedes the earlier "no tendril-side fix" conclusion on bd-acf75f. If a future tendril workflow adds a Nix job, target that same label set (or `nixos`), never `azure-ephemeral`.
