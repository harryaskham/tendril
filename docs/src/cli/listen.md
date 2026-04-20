# `tendril listen`

Use `tendril listen` to capture audio from the current platform.

## Example

```bash
# Default: write a temp WAV path and print the path in the JSON envelope.
tendril listen --json --source system --duration-ms 5000 --format wav

# Or save directly to a file (mirrors `capture -o`).
tendril listen --source microphone --duration-ms 3000 -o /tmp/mic.wav
```

## Current implementation

`listen` now performs a real recording on supported backends and falls back
to probe-only diagnostics elsewhere:

- **Linux + PipeWire**: prefers `pw-record` (with `parecord` as a fallback).
  PipeWire's `--target` plus `-n <samples>` is used so the recorder exits on
  its own and the WAV header is always finalized.
- **Linux + PulseAudio**: uses `parecord` against `@DEFAULT_MONITOR@` /
  `@DEFAULT_SOURCE@`.
- **macOS**: uses `afrecord` (Apple's CoreAudio-backed recorder shipped with
  the OS) with `-d <seconds>` so it stops at the requested duration.
- **Windows / unknown backends**: capture is not yet wired; the JSON envelope
  reports `status = "probe_only"` with a structured note explaining the gap.

When a recording succeeds, the response includes an `execution.artifact`
object with the on-disk path, byte size, sample rate, channel count, and the
recorder program that produced the file. If the artifact contains only the
44-byte WAV header (no PCM samples), `notes` includes a warning so callers
can detect silent sessions (suspended monitor, muted mic, no audio playing).

## Output paths

- `--output <path>` / `-o <path>`: write the WAV to an explicit path. The
  parent directory must already exist; this is validated before any recorder
  is spawned.
- Omitted: `listen` allocates a temp file under the system temp directory
  (e.g. `/tmp/tendril-listen-<pid>-<nanos>.wav`) and reports its path in the
  JSON envelope.

## Supported source selectors

- `system` / `loopback` — default monitor of the active sink.
- `microphone` / `mic` — default input source.
- `device:<id>` — modeled in the surface but currently returns a structured
  `audio_device_selection_not_implemented` result. Real per-device binding
  remains a follow-up.

## Format support

- `wav` — implemented.
- `flac`, `opus` — accepted by the CLI but currently degrade to probe-only;
  WAV is the only emitted artifact format today.
