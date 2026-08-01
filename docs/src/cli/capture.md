# `tendril capture`

Use `tendril capture` to request a screenshot for a selected display/window or one frame from a camera discovered by `tendril list`.

## Example

```bash
tendril --window <id> capture --json --max-width 1280 --format png --compression 90

# Camera ids are listed under the `cameras` array/section.
tendril list --json
tendril --camera <id> capture -o camera.png
```

## Current flags

- `--max-width <pixels>`
- `--max-height <pixels>`
- `--format <png|jpeg>`
- `--compression <0-100>`
- `--timeout-ms <milliseconds>`
- `-o, --output <path>` — write the decoded image directly to a file. This
  flag is side-effecting: when combined with `--json` the JSON envelope is
  still printed to stdout while the image bytes are written to `<path>`.

## Save to file

```bash
# Save a PNG screenshot to disk (no JSON envelope on stdout)
tendril --window <id> capture -o /tmp/screen.png

# Save to disk and also emit the JSON envelope on stdout
tendril --json --window <id> capture -o /tmp/screen.png
```

## Camera backends

Camera capture uses ffmpeg with the native desktop video-input backend:

- **macOS:** AVFoundation. Tendril explicitly requests 30 fps rather than ffmpeg's incompatible 29.97 fps default and retries at an advertised device rate when necessary.
- **Linux:** V4L2. Camera ids are `/dev/videoN` device nodes discovered from `/sys/class/video4linux`; an unambiguous friendly name is also accepted.
- **Windows:** DirectShow. Tendril discovers friendly names and stable alternative device ids from ffmpeg's DirectShow inventory.

Install `ffmpeg`/`ffmpeg.exe` on `PATH` when it is not supplied by the package. Camera permission and device-node/privacy settings remain platform-managed.

## Response shape

Window/display JSON output includes the selected target, original/output bounds, explicit coordinate transforms, resize state, image metadata, and a base64 payload. Camera output instead includes the device id, width, height, format, media type, capture time, and base64 payload; coordinate transforms do not apply to a camera frame.

## Config interaction

If a flag is omitted, Tendril falls back to values from `config.yaml` for capture format, compression, and optional max dimensions.

See [Configuration](../reference/configuration.md) for defaults.
