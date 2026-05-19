# Android device backend

Tendril can drive a connected Android device or emulator through ADB:

```bash
tendril --android sgu24:5555 list --json
tendril --android emulator-5554 list-elements --json
tendril --android auto capture -o screenshot.png
tendril --android sgu24:5555 run 'click(231,1905),wait(500ms),press("Monitor")'
```

`--android auto` selects the single connected `adb devices` entry and fails clearly when zero or multiple devices are available. If the flag is omitted, `TENDRIL_ANDROID_SERIAL` provides the same serial selection.

## Supported MVP commands

- `list` checks `adb get-state`, reads device metadata such as `wm size`, `wm density`, model, and focused window/activity, and returns an Android display target named `android:<serial>`.
- `list-elements` runs `adb shell uiautomator dump /sdcard/tendril-window.xml`, reads the XML back with `adb exec-out cat`, and maps UIAutomator nodes into Tendril element descriptors with text/content-desc/resource-id/class/package/bounds and clickability flags.
- `capture` runs `adb exec-out screencap -p` and returns the PNG in Tendril's normal capture envelope. `-o/--output` writes the decoded PNG.
- Every Android invocation writes debug artifacts under `TENDRIL_ANDROID_ARTIFACT_DIR` when set, or a per-run directory under the system temp directory. The MVP writes `commands.log`, `ui.xml` for element dumps, `screenshot.png` for captures, and `window.txt` when focus metadata is available.
- `run` maps Tendril's existing DSL onto `adb shell input` primitives:
  - `click(x,y)` / `lclick(x,y)` → `input tap`
  - `dblclick(x,y)` → two taps
  - `drag(x0,y0,x1,y1)` and `scroll(x,y,dy)` → `input swipe`
  - `send("text")` or plain text → `input text`
  - `Return`, `Back`, `Home`, `Wakeup`, `Menu`, `Tab`, `Space`, `Escape`, and numeric keyevent codes → `input keyevent`
  - `click(<element-id>)` can target IDs returned by `list-elements`, or exact text/content-desc/resource-id values from the current UI dump.
  - `press("launch:<package>")` or `press("package:<package>")` launches an app through `monkey -p <package> -c android.intent.category.LAUNCHER 1`.

## Safety model

The MVP backend is intentionally limited to ordinary UI-driving primitives. It does not factory reset, uninstall, clear app data, force-stop unrelated packages, or kill processes. Explicit app launch via `press("launch:<package>")` is allowed; richer semantic selectors are planned follow-ups.

## Debugging notes

The backend surfaces ADB failures as structured Tendril errors. Common causes are:

- `adb` is missing from `PATH`.
- The serial is offline or unauthorized.
- `--android auto` found zero or multiple devices.
- UIAutomator cannot dump the current surface, for example on some lockscreen/system overlay states.

For robust workflows, start with `list --json`, then `list-elements --json`, then use element IDs or coordinate taps from the observed bounds.
