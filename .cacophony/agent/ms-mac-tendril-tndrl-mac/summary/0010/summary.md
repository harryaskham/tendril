# Session summary — listen.rs recorder-selection + exit-acceptance test coverage

## Goal

Idle-cycle incremental coverage work: with the tendril board clear of open beads
and no active claims, add deterministic, macOS-host-validatable unit tests for
under-tested pure helpers in `listen.rs` that govern audio capture recorder
selection and the SIGTERM-kill exit contract.

## Bead(s)

- `bd-2da4ce` — Add unit coverage for listen.rs macOS recorder selection and exit-acceptance helpers

## Before state

- Failing tests: none
- `listen.rs`: 8 tests; helpers `build_afrecord_args`, `recorders_for`,
  `is_acceptable_exit`, `program_runs_until_killed` had no direct unit coverage
- tendril lib tests: 232 passing

## After state

- Failing tests: none
- `listen.rs`: 15 tests (+7)
- tendril lib tests: 239 passing; clippy `-D warnings` clean (only the
  pre-existing benign `ashpd v0.8.1` future-incompat note, unrelated)

## Diff summary

- Code/content commit: pending final squash SHA from reintegration receipt
- Files touched: `crates/tendril/src/listen.rs` (test module only)
- Tests: +7 / -0 / flipped 0
  - `afrecord_args_request_time_bounded_wav` — macOS afrecord argv emits
    `-f WAVE`, `-d LEI16`, `-t <duration>`, output path last
  - `recorders_for_selects_afrecord_on_macos`
  - `recorders_for_uses_parecord_for_pulseaudio`
  - `recorders_for_prefers_pw_record_on_unknown_linux_backend`
  - `recorders_for_is_empty_on_windows_and_android`
  - `only_parecord_runs_until_killed`
  - `nonzero_exit_is_only_acceptable_for_run_until_killed_recorders` (unix-gated)
- Behavioural delta: none — test-only change

## Operator-takeaway

`listen.rs` recorder selection is now pinned by tests: macOS uses `afrecord`
at 44.1kHz/1ch, Linux prefers `pw-record` then falls back to `parecord`, and
Windows/Android have no recorder. The exit-acceptance contract is also locked:
only `parecord` is treated as run-until-killed, so a non-zero exit is the
normal post-SIGTERM path for it but a real error for the self-terminating
recorders. This guards against silent drift if recorder programs/flags change.
