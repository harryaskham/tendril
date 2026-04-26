# bd-1f08f8 X11 black window capture validation

Environment: `scripts/tendril-headless.sh --name bd-1f08f8` with `./target/debug/tendril` on Xvfb/Openbox/Chromium/XTerm.

Repro/explanation:
- Controller artifact `summaries/tndl-ctrl/headless-scroll-browser-os-20260426T101949Z/tndl-ctrl-scroll-20260426T101949Z-xterm-os-after.png` decodes as a 604x316 RGBA PNG with zero non-black pixels at the same threshold used by the fix. Its JSON capture envelope reported success, confirming the bug was a successful but unusable X11 window-drawable capture.
- The X11 path used XGetImage on the target window drawable. In this headless X11/Openbox scenario, after input restores focus to the foreground browser, an obscured XTerm can expose no usable backing pixels to that path, yielding an all-black PNG.

Fixed validation:
- `list.json` discovered the foreground Chromium window and the XTerm window.
- `xterm-run.json` sent an XTerm shell command while restore-focus was enabled; `os-side-effect/result.txt` proves the command executed.
- `xterm-after-fixed-capture.json` is a successful post-restore capture of the XTerm target using the fixed binary.
- `xterm-after-fixed-stats.json` reports 136857 / 137216 non-black output pixels, so Tendril did not silently return another solid-black success.

Implementation behavior:
- X11 window capture still tries the target window drawable first.
- If that decoded PNG is solid black, Tendril raises the target, captures the target bounds from the root window, validates the fallback is not solid black, then restores the previous X11 focus/pointer state.
- If either path cannot produce usable pixels, Tendril returns a platform adapter diagnostic instead of a black success.
