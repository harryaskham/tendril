# Session summary — input.rs ensure_input_supported gate coverage

## Goal

Idle-cycle incremental coverage work: with the tendril board clear of open
beads, pin the sibling of capture.rs::ensure_capture_supported — namely
input.rs::ensure_input_supported, the pre-execution capability gate for input
actions. The capture sibling landed in bd-4b80c0 last cycle; pinning the
input sibling closes the symmetry. Host-validatable on macOS.

## Bead(s)

- `bd-025bf6` — Add unit coverage for input.rs ensure_input_supported gate

## Before state

- Failing tests: none
- ensure_input_supported had no direct test; the sibling
  ensure_capture_supported was already pinned in bd-4b80c0
- tendril lib tests: 316 passing

## After state

- Failing tests: none
- tendril lib tests: 317 passing (+1); clippy `-D warnings` clean (only the
  pre-existing benign `ashpd v0.8.1` future-incompat note, unrelated)

## Diff summary

- Code/content commit: pending final squash SHA from reintegration receipt
- Files touched: `crates/tendril/src/input.rs` (test module only; added
  ensure_input_supported to the super:: import)
- Tests: +1 / -0 / flipped 0
  - `ensure_input_supported_gates_on_the_capability_flag`: a descriptor with
    input_supported = true yields Ok(()); flipping it to false yields Err with
    code input_not_supported_for_target and a target_id detail equal to the
    descriptor id. Reuses the existing browser_target test helper
- Behavioural delta: none — test-only change

## Operator-takeaway

The input capability gate is now pinned alongside the capture gate: a target
must have input_supported = true to be admitted to input execution, and a
disallowed target is rejected with a structured input-not-supported error
naming the offending target id. With both gates pinned, the capture/input
admission symmetry is guaranteed to stay intact across refactors. Lib tests
now 317, up from 207 this session.
