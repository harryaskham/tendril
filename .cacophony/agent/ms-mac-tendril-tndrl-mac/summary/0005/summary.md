# Session summary — Android ADB backend MVP

## Goal

Add first-class Android-device driving support to Tendril so agents can inspect and drive real Android devices/emulators through ADB and Tendril's existing list/capture/run workflow instead of ad-hoc shell snippets.

## Bead(s)

- `bd-69cdaa` — Add first-class Android device backend for Tendril
- Follow-up filed: `bd-ee984b` — Add richer Android semantic selector DSL

## Before state

- Failing tests: none known.
- Relevant metrics: Tendril supported desktop/window backends only; Android workflows required raw `adb`, `uiautomator dump`, `input tap`, and manual parsing outside Tendril.
- Context: The bead requested an MVP that could select devices by serial/auto/env, observe UIAutomator nodes, capture screenshots, dispatch basic UI input, launch apps, and record useful artifacts.

## After state

- Failing tests: no compile failures in the final code path; real-device smoke was not run from this macOS checkout.
- Relevant metrics: `cargo check -p tendril --tests --offline --message-format short` passed; queued build `bj-317f44a0` passed for `nix build .#checks.aarch64-darwin.fmt .#checks.aarch64-darwin.docs --no-link`. Direct `cargo test -p tendril android::tests --lib --offline` reached the linker but failed outside the Nix dev environment because `ld` could not find `-liconv`; earlier queued cargo/Nix package builds also stalled behind shared build activity and were canceled/timed out.
- Context: Tendril now has a `--android <serial|auto>` global backend, honors `TENDRIL_ANDROID_SERIAL`, writes Android debug artifacts, and documents supported commands and safety limits.

## Diff summary

- Code/content commits: `6761af5` and `56c3632` (`bd-69cdaa: add Android adb backend MVP`)
- Summary artefact commit: intentionally omitted; this file must not self-reference its own mutable SHA
- Files touched: `crates/tendril/src/android.rs`, `crates/tendril/src/cli.rs`, `crates/tendril/src/commands/mod.rs`, `crates/tendril/src/discovery.rs`, `crates/tendril/src/lib.rs`, `crates/tendril/src/listen.rs`, `crates/tendril/src/platform.rs`, `README.md`, `docs/src/SUMMARY.md`, `docs/src/reference/android.md`
- Tests: added 3 Android parser/escaping unit tests; no existing tests removed or flipped.
- Behavioural delta: `list`, `list-elements`, `capture`, and `run` can now operate against an Android device via ADB. UIAutomator XML nodes become Tendril element descriptors; screenshots use `screencap`; input maps to `adb shell input`; `press("launch:<package>")` launches an app; per-run artifacts include `commands.log`, `ui.xml`, `screenshot.png`, and `window.txt` when available.

## Operator-takeaway

The Android workflow Harry called out is now a Tendril backend MVP rather than a pile of one-off ADB commands; richer natural selector syntax is intentionally split into `bd-ee984b` so the foundation can land first.
