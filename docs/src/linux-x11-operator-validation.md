# Linux/X11 operator validation

Use this guide when you want to validate a packaged Tendril binary on a real Linux host running an active X11 session.

This smoke path is specifically intended to catch the historical packaged-binary failure mode where `tendril list`, `capture`, or `run` depended on extra package-manager tools such as `xrandr`, `xprop`, `xwininfo`, `import`, or `xdotool`.

## Preconditions

- Linux host
- active local X11 session
- `DISPLAY` set
- packaged Tendril artifact available locally, or the ability to build/stage it with Nix

The script below refuses to run on Wayland because it is intended to validate the generic X11 backend only.

## Minimal packaged smoke check

From the repository root:

```bash
./scripts/linux-x11-packaged-smoke.sh
```

The script:

1. stages `.#releaseArtifact` when needed,
2. extracts the packaged `tendril` binary,
3. runs `tendril --json list` inside the active X11 session,
4. verifies the output stays structured and stateless,
5. checks that no missing-tool diagnostics mention `xrandr`, `xprop`, or `xwininfo`, and
6. captures the first discovered display and checks that no `import` dependency remains.

A successful run ends with output like:

```text
Verified packaged Linux/X11 tendril list+capture smoke coverage via .../tendril-0.0.1-x86_64-linux.tar.gz
```

## Optional packaged `run` smoke

Input injection is real and can affect the selected window, so the packaged `run` smoke is opt-in.

Choose a disposable window target from `tendril list --json`, then run:

```bash
TENDRIL_X11_SMOKE_RUN_TARGET=0x123456 \
TENDRIL_X11_SMOKE_RUN_SEQUENCE='send("smoke")' \
./scripts/linux-x11-packaged-smoke.sh
```

The script will additionally verify that packaged input execution succeeds without surfacing `xdotool` runtime failures.

## Manual spot checks

If you prefer to drive the binary directly, extract the packaged artifact and run:

```bash
tendril --json list
```

Then capture a display from the returned target list:

```bash
tendril --display <display-id> --json capture
```

Expected packaged-flow properties:

- `meta.command` matches the invoked command,
- `data.adapter.platform` is `linux`,
- `data.adapter.session` is `x11`,
- `data.adapter.stateless` is `true`, and
- failures do **not** mention missing `xrandr`, `xprop`, `xwininfo`, `import`, or `xdotool` helpers.

## Troubleshooting

If the smoke script exits immediately:

- check that `DISPLAY` is set,
- confirm `echo "$XDG_SESSION_TYPE"` reports `x11`, and
- rerun from a local graphical login session instead of a headless shell.

If `list` or `capture` still fails in a real X11 session, capture the JSON envelope plus stderr and file a follow-up bead with the exact X server and window-manager details.
