# Headless Tendril micro-environment

Tendril ships a Linux/X11 helper for agent browser and OS-control work that must
not touch the operator's real desktop. It starts a disposable 1920x1080 Xvfb
session with a lightweight window manager, browser, and optional shell window,
then points ordinary `tendril list`, `tendril capture`, and `tendril run`
commands at that isolated display.

Use this micro-environment when an agent needs to click, type, navigate a
browser, or capture UI state without stealing focus from the human operator.
Use the real desktop only when the task explicitly needs the operator's current
session, native window manager, hardware acceleration, or platform permission
prompts.

## What it provides

- Default deterministic desktop: `1920x1080x24`, scale factor 1:1.
- Isolated `DISPLAY` chosen from `:90`-`:199` unless `--display` is supplied.
- `Xvfb` with XTEST enabled, so Tendril's X11 input backend can type/click.
- Lightweight window manager when available (`openbox`, `fluxbox`,
  `matchbox-window-manager`, or `twm`).
- Browser when available (`chromium`, `chromium-browser`, `google-chrome`, or
  `firefox`) plus basic shell utilities.
- Resource guardrails for the default Chromium path: fixed-size Xvfb screen,
  disabled background/sync/extension work, and a renderer process cap of 2
  (`--browser-renderers <n>` to tune, `0` to omit).
- Clean lifecycle commands: `start`, `env`, `inspect`, `reset`, `stop`, and
  `smoke`.
- Smoke artifacts written to a git-trackable summaries directory by default:
  `summaries/$CACOPHONY_AGENT/` or `summaries/manual/`.

The helper is intentionally process-scoped and disposable. It is not a Tendril
daemon and it does not add hidden Tendril state. The only state file is the
helper's lifecycle metadata under `$XDG_RUNTIME_DIR/tendril-headless/<name>/` so
subsequent `env`, `inspect`, `reset`, and `stop` commands can find the sandbox.

## Copy-paste smoke test

From a checkout, build the latest Tendril binary and run the smoke workflow:

```bash
cargo build -p tendril
scripts/tendril-headless.sh --name smoke --tendril-bin ./target/debug/tendril smoke
```

If you want to force execution through the latest Nix package output instead of
a system Tendril binary, run the packaged helper directly:

```bash
nix run .#tendril-headless -- --name smoke smoke
```

You can also keep using the checkout script while routing Tendril CLI calls
through Nix explicitly:

```bash
scripts/tendril-headless.sh \
  --name smoke \
  --tendril-bin 'nix run .#tendril --' \
  smoke
```

What the smoke does:

1. Starts or reuses an isolated Xvfb desktop.
2. Waits until `tendril --json list` sees a 1920x1080 display and a browser
   window.
3. Captures the display with explicit `--max-width 1920 --max-height 1080`.
4. Runs a browser-focused input sequence:
   `hold(ctrl),l,release(ctrl),send("tendril headless smoke"),return`.
5. Writes artifacts under `summaries/<agent-id>/` and runs `git add` for that
   directory unless `--no-git-add-artifacts` is set.
6. Stops the sandbox when the smoke command started it.

Expected artifacts:

```text
summaries/<agent-id>/smoke-list.json
summaries/<agent-id>/smoke-capture.json
summaries/<agent-id>/smoke-run.json
summaries/<agent-id>/smoke-display.png
summaries/<agent-id>/smoke-manifest.txt
```

The `.png` is deliberately in `summaries/` rather than `/tmp`; Cacophony summary
collectors and `/tmp/watch-captures.sh` can then surface the capture to the
operator.

## Manual lifecycle

Start an environment and export its display into the current shell:

```bash
eval "$(scripts/tendril-headless.sh --name browser-task start)"
```

Inspect targets without touching the real desktop:

```bash
tendril --json list
```

Capture the isolated display into the summaries directory:

```bash
mkdir -p "summaries/${CACOPHONY_AGENT:-manual}"
display_id=$(tendril --json list | python3 -c '
import json, sys
for target in json.load(sys.stdin)["data"]["targets"]:
    if target["kind"] == "display":
        print(target["id"])
        break
')
tendril --json --display "$display_id" capture \
  --max-width 1920 --max-height 1080 \
  -o "summaries/${CACOPHONY_AGENT:-manual}/browser-task-display.png"
git add "summaries/${CACOPHONY_AGENT:-manual}/browser-task-display.png"
```

Run input against a discovered browser window:

```bash
window_id=$(tendril --json list | python3 -c '
import json, sys
targets=json.load(sys.stdin)["data"]["targets"]
for target in targets:
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if target["kind"] == "window" and any(token in haystack for token in ("chrom", "firefox")):
        print(target["id"])
        break
')
tendril --json --window "$window_id" run \
  'hold(ctrl),l,release(ctrl),send("https://example.com"),return'
```

Reset or stop the environment:

```bash
scripts/tendril-headless.sh --name browser-task reset
scripts/tendril-headless.sh --name browser-task stop
```

## Resource and safety notes

- Use a unique `--name` per agent/task so multiple workers do not share runtime
  directories or display numbers accidentally.
- Keep the Chromium renderer cap low for routine agent tasks; raise it only for
  pages that genuinely need more renderer processes.
- `start` reuses a live environment with the same name; `reset` destroys and
  recreates it.
- Runtime logs stay under `$XDG_RUNTIME_DIR/tendril-headless/<name>/logs/` and
  are removed by `stop` unless `--keep-runtime` is supplied.
- Smoke capture artifacts are refused if `--artifact-dir` points at `/tmp`,
  `/var/tmp`, or `/run`; use a checkout-local summaries path for captures.
- The Nix package installs both `tendril` and `tendril-headless`; release
  archives include both binaries.
- The helper is Linux/X11-first. On macOS and Windows, use the native Tendril
  validation guides or a future VM/microVM sidecar when stronger isolation is
  required.
