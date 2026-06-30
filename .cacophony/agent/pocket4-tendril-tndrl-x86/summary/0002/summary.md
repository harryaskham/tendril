# Session summary — Correct stale audio-capture status in PROJECT_HEALTH.md

## Goal

While the tendril board was drained of open work, the operator asked for
meaningful self-directed progress. A read of `PROJECT_HEALTH.md` against current
source showed its audio-capture status was stale: it still described
`tendril listen` as probe-only with "artifact emission not yet implemented",
even though real WAV capture has since landed. This session corrects that
handoff documentation so future agents and operators do not chase
already-completed work.

## Bead(s)

- `bd-c6617f` — PROJECT_HEALTH.md: correct stale audio-capture status (listen now emits real WAV on Linux + macOS)

Adjacent bead filed this session (not worked here, macOS-only):
- `bd-5110d9` — Ship tendril as a signed macOS .app bundle so it can self-request TCC permissions (feature, left unclaimed for a macOS node)

## Before state

- Failing tests: none touched (docs-only change).
- `PROJECT_HEALTH.md` Surface map listed `Audio capture | Partial by design | tendril listen is probe-first ... artifact emission as not yet implemented`.
- Known follow-up #1 read "Finish audio artifact capture beyond capability probing" as if unstarted.
- Operator-facing status said remaining work was "full audio capture and automated coverage enforcement".
- Reality in `crates/tendril/src/listen.rs` (`execute_listen_capture` / `recorders_for`): real WAV capture is wired for Linux PipeWire (`pw-record`->`parecord`), Linux PulseAudio (`parecord`), and macOS (`afrecord` + ffmpeg/avfoundation loopback, bd-d92c7e). `probe_only` is reserved for Windows/Android and non-WAV formats. `README.md` already documents this accurately (L393/L412).

## After state

- Failing tests: none (docs-only).
- `PROJECT_HEALTH.md` Surface map row now reads "Ready on Linux + macOS; probe-only on unwired lanes" with concrete evidence.
- Known follow-up #1 reframed to "Extend audio artifact capture to the remaining platform lanes" with an accurate Status line; remaining bounded work = Windows/Android lanes + non-WAV formats.
- Operator-facing status updated to reflect Linux+macOS capture is implemented.
- Checkout rebased onto current `origin/main`; an unrelated `crates/mcp-cli` submodule rewind from the rebase was restored to the recorded pointer (941015b) so the commit is docs-only.

## Diff summary

- Code/content commit: `540e64d` (bd-c6617f). Final landed squash SHA will come from the reintegration receipt.
- Files touched: `PROJECT_HEALTH.md` (3 hunks: Surface map row, Known follow-up #1, operator-facing status paragraph).
- Tests: +0 / -0 / flipped 0 (documentation only).
- Behavioural delta: none; documentation now matches shipped `tendril listen` behaviour.

## Operator-takeaway

`tendril listen` real audio capture is already done for Linux and macOS — only
Windows/Android lanes and non-WAV formats remain (correctly `probe_only`).
PROJECT_HEALTH.md had drifted behind the code and was over-stating the remaining
audio work; it now matches `listen.rs` and `README.md`. Separately, the macOS
"self-request TCC permission" question is captured as feature bead bd-5110d9 for
a macOS-capable worker (tendril is an unsigned CLI today, so consent binds to the
parent launcher; a signed .app bundle is the real fix).
