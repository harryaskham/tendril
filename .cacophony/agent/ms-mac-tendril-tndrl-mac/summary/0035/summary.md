# Session summary — input.rs validate_relative_point bounds coverage

## Goal

Idle-cycle incremental coverage work: with the tendril board clear of open beads
and no other claims, pin validate_relative_point in input.rs — the half-open
relative-coordinate accept box and its four out-of-bounds rejection edges with
structured error details — which the existing relative_point_to_absolute mapping
tests did not exercise. Host-validatable on macOS.

## Bead(s)

- `bd-9002b2` — Add unit coverage for input.rs validate_relative_point bounds checking

## Before state

- Failing tests: none
- validate_relative_point had no direct test; the relative-coordinate mapping
  helpers (relative_point_to_absolute) were covered but not the bounds rejection
  path
- tendril lib tests: 305 passing

## After state

- Failing tests: none
- tendril lib tests: 306 passing (+1); clippy `-D warnings` clean (only the
  pre-existing benign `ashpd v0.8.1` future-incompat note, unrelated)

## Diff summary

- Code/content commit: pending final squash SHA from reintegration receipt
- Files touched: `crates/tendril/src/input.rs` (test module only; added
  validate_relative_point to the super:: import)
- Tests: +1 / -0 / flipped 0
  - `validate_relative_point_accepts_interior_and_rejects_out_of_bounds`:
    accepts the inclusive lower corner (0,0), the inclusive upper interior corner
    (width-1, height-1), and a middle point; rejects x<0, y<0, x==width, y==height
    (half-open upper bound), each with code invalid_run_input and details
    stage=validate, field, action_index, action_number == action_index + 1
- Behavioural delta: none — test-only change

## Operator-takeaway

The relative-pointer bounds guard is now pinned: a relative click/move point is
accepted only inside the half-open [0,width) x [0,height) box, and every
out-of-range edge is rejected with the structured validate-stage details
(field + 1-based action_number) the DSL surfaces to callers. This protects the
relative-coordinate input path from off-by-one boundary drift. Lib tests now 306,
up from 207 this session.
