# Session summary — Nix runtime dependency audit

## Goal

Audit Tendril's Nix package dependency surface for build/profile path collisions and make the Linux wrappers expose only the helper commands Tendril actually invokes, rather than whole dependency `bin/` directories.

## Bead(s)

- `bd-72c11b` — Audit tendril build dependencies for package conflicts
- Related triage: `bd-ec32fc` was marked draft/out-of-repo after confirming the reported `agent-utils` versus `iproute2` `ss` conflict is not fixable in Tendril source; duplicate `bd-3996db` was linked to it.

## Before state

- Failing tests: none known at checkout start.
- Relevant metrics: Tendril's Linux Nix wrapper used `lib.makeBinPath` over whole helper packages (`hyprland`, `sway`, `wlr-randr`, `grim`, `ydotool`, `wtype`, and headless helpers), which exposed every binary in those package outputs on wrapper `PATH`.
- Context: The triggering `ss` conflict came from host/profile composition (`agent-utils` and `iproute2`) rather than a Tendril output, but the wrapper dependency surface was still broad enough to deserve tightening.

## After state

- Failing tests: none observed in focused validation.
- Relevant metrics: `nix flake check --no-build` evaluates successfully; queued build `bj-e2602d9a` succeeded for `nix build .#checks.aarch64-darwin.fmt .#checks.aarch64-darwin.docs .#checks.aarch64-darwin.linuxRuntimeDependencyAudit --no-link`.
- Context: Tendril now builds curated wrapper PATH directories from command-level declarations and has a `linuxRuntimeDependencyAudit` check that records command/package/executable mappings and fails on duplicate command names.

## Diff summary

- Code/content commits: `c24dd9b` (`bd-72c11b: audit nix runtime dependencies`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `flake.nix`, `docs/src/SUMMARY.md`, `docs/src/reference/nix-runtime-dependency-audit.md`
- Tests: +0 / -0 / flipped 0
- Behavioural delta: Linux wrappers no longer put broad dependency package `bin/` directories on `PATH`; they link only declared commands such as `hyprctl`, `swaymsg`, `grim`, `ydotool`, `wtype`, and the headless helper commands. The new docs state that Tendril does not expose `ss` or depend on `iproute2` directly.

## Operator-takeaway

The original `agent-utils`/`iproute2` `ss` conflict is not a Tendril package bug, but this session reduced Tendril's future collision risk by making its Nix wrapper dependency surface explicit, auditable, and checked.
