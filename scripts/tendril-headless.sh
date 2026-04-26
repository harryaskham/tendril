#!/usr/bin/env bash
set -euo pipefail

DEFAULT_NAME="default"
DEFAULT_WIDTH="1920"
DEFAULT_HEIGHT="1080"
DEFAULT_DEPTH="24"
DEFAULT_BASE_DIR="${XDG_RUNTIME_DIR:-/tmp}/tendril-headless"
DEFAULT_ARTIFACT_DIR="${TENDRIL_HEADLESS_ARTIFACT_DIR:-summaries/${CACOPHONY_AGENT:-${CACO_AGENT_ID:-manual}}}"

NAME="$DEFAULT_NAME"
WIDTH="$DEFAULT_WIDTH"
HEIGHT="$DEFAULT_HEIGHT"
DEPTH="$DEFAULT_DEPTH"
BASE_DIR="$DEFAULT_BASE_DIR"
ARTIFACT_DIR="$DEFAULT_ARTIFACT_DIR"
DISPLAY_NUMBER=""
BROWSER_RENDERER_LIMIT="${TENDRIL_HEADLESS_BROWSER_RENDERER_LIMIT:-2}"
BROWSER_STARTUP_GRACE="${TENDRIL_HEADLESS_BROWSER_STARTUP_GRACE:-3}"
CHROMIUM_SANDBOX="${TENDRIL_HEADLESS_CHROMIUM_SANDBOX:-false}"
KEEP_RUNTIME="false"
GIT_ADD_ARTIFACTS="${TENDRIL_HEADLESS_GIT_ADD_ARTIFACTS:-true}"
TENDRIL_BIN="${TENDRIL_HEADLESS_TENDRIL_BIN:-tendril}"
BROWSER_BIN="${TENDRIL_HEADLESS_BROWSER:-}"
COMMAND=""
UPLOAD_FILE=""
FILE_INPUT_SELECTOR='input[type="file"]'
NAVIGATE_URL=""
HELPER_OUTPUT=""
MARIONETTE_PORT="${TENDRIL_HEADLESS_MARIONETTE_PORT:-}"

usage() {
  cat <<'USAGE'
Usage: scripts/tendril-headless.sh [options] <command>

Commands:
  start      Start or reuse an isolated 1920x1080 X11 desktop and print exports.
  env        Print exports for a running environment.
  inspect    Show lifecycle state and process health.
  reset      Stop then start the environment.
  stop       Stop processes and remove the runtime directory.
  smoke      Run Tendril list/capture/run against the isolated desktop.
             When XTerm is available, also dispatch Shift+Insert to prove the
             X11 backend supports the standard terminal paste shortcut.
  firefox-upload
             Set a file input in the running headless Firefox via Marionette.
  file-upload-smoke
             Reproduce the native Firefox chooser limitation, then upload via helper.

Options:
  --name <name>          Environment name (default: default).
  --runtime-dir <dir>    Base runtime directory (default: $XDG_RUNTIME_DIR/tendril-headless or /tmp/tendril-headless).
  --display <number>     X display number without ':'; otherwise auto-pick an unused display.
  --width <px>           Desktop width (default: 1920).
  --height <px>          Desktop height (default: 1080).
  --depth <bits>         Xvfb screen depth (default: 24).
  --browser <path>       Browser binary; otherwise auto-detect chromium/google-chrome/firefox.
                         Explicit browsers disable automatic browser fallback.
  --tendril-bin <path>   Tendril binary for smoke (default: tendril or $TENDRIL_HEADLESS_TENDRIL_BIN).
  --artifact-dir <dir>   Smoke artifact directory (default: summaries/$CACOPHONY_AGENT or summaries/manual).
  --browser-renderers <n> Chromium renderer process cap (default: 2; set 0 to omit the flag).
                         Chromium sandboxing is disabled by default in this
                         disposable Xvfb sandbox; set TENDRIL_HEADLESS_CHROMIUM_SANDBOX=true to keep it.
  --marionette-port <n>  Firefox Marionette port for browser helpers (default: auto-pick localhost port).
  --upload-file <path>   Local file path for firefox-upload.
  --file-input-selector <css>
                         CSS selector for firefox-upload (default: input[type="file"]).
  --navigate-url <url>   Optional URL to load before firefox-upload selects the file.
  --helper-output <path> Write firefox-upload JSON to a file instead of stdout.
  --no-git-add-artifacts Do not run git add for smoke artifacts.
  --keep-runtime         Keep runtime files on stop.
  -h, --help             Show this help.

Copy-paste smoke from a built checkout:
  cargo build -p tendril
  scripts/tendril-headless.sh --tendril-bin ./target/debug/tendril smoke

Copy-paste smoke using the latest checkout through Nix:
  scripts/tendril-headless.sh --tendril-bin 'nix run .#tendril --' smoke

Firefox file-upload smoke:
  scripts/tendril-headless.sh --browser firefox --tendril-bin ./target/debug/tendril file-upload-smoke
USAGE
}

log() {
  printf '[tendril-headless] %s\n' "$*" >&2
}

fail() {
  log "error: $*"
  exit 1
}

have() {
  command -v "$1" >/dev/null 2>&1
}

quote() {
  printf '%q' "$1"
}

port_available() {
  python3 - "$1" <<'PY'
import socket
import sys
port = int(sys.argv[1])
with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        sock.bind(("127.0.0.1", port))
    except OSError:
        raise SystemExit(1)
PY
}

choose_marionette_port() {
  local display_number="$1" candidate
  if [[ -n "$MARIONETTE_PORT" ]]; then
    printf '%s' "$MARIONETTE_PORT"
    return 0
  fi

  candidate=$((62000 + display_number))
  if [[ "$candidate" -le 65535 ]] && port_available "$candidate"; then
    printf '%s' "$candidate"
    return 0
  fi

  for candidate in $(seq 62090 62250); do
    if port_available "$candidate"; then
      printf '%s' "$candidate"
      return 0
    fi
  done

  fail "could not find a free localhost port for Firefox Marionette"
}

abspath() {
  python3 -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$1"
}

dsl_escape() {
  python3 -c 'import json, sys; print(json.dumps(sys.argv[1])[1:-1])' "$1"
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      start|env|inspect|reset|stop|smoke|firefox-upload|file-upload-smoke)
        if [[ -n "$COMMAND" ]]; then
          fail "only one command may be provided"
        fi
        COMMAND="$1"
        shift
        ;;
      --name)
        NAME="${2:-}"
        [[ -n "$NAME" ]] || fail "--name requires a value"
        shift 2
        ;;
      --runtime-dir)
        BASE_DIR="${2:-}"
        [[ -n "$BASE_DIR" ]] || fail "--runtime-dir requires a value"
        shift 2
        ;;
      --display)
        DISPLAY_NUMBER="${2:-}"
        [[ "$DISPLAY_NUMBER" =~ ^[0-9]+$ ]] || fail "--display requires a numeric display number"
        shift 2
        ;;
      --width)
        WIDTH="${2:-}"
        [[ "$WIDTH" =~ ^[0-9]+$ ]] && [[ "$WIDTH" -gt 0 ]] || fail "--width requires a positive integer"
        shift 2
        ;;
      --height)
        HEIGHT="${2:-}"
        [[ "$HEIGHT" =~ ^[0-9]+$ ]] && [[ "$HEIGHT" -gt 0 ]] || fail "--height requires a positive integer"
        shift 2
        ;;
      --depth)
        DEPTH="${2:-}"
        [[ "$DEPTH" =~ ^[0-9]+$ ]] && [[ "$DEPTH" -gt 0 ]] || fail "--depth requires a positive integer"
        shift 2
        ;;
      --browser)
        BROWSER_BIN="${2:-}"
        [[ -n "$BROWSER_BIN" ]] || fail "--browser requires a value"
        shift 2
        ;;
      --tendril-bin)
        TENDRIL_BIN="${2:-}"
        [[ -n "$TENDRIL_BIN" ]] || fail "--tendril-bin requires a value"
        shift 2
        ;;
      --artifact-dir)
        ARTIFACT_DIR="${2:-}"
        [[ -n "$ARTIFACT_DIR" ]] || fail "--artifact-dir requires a value"
        shift 2
        ;;
      --browser-renderers)
        BROWSER_RENDERER_LIMIT="${2:-}"
        [[ "$BROWSER_RENDERER_LIMIT" =~ ^[0-9]+$ ]] || fail "--browser-renderers requires a non-negative integer"
        shift 2
        ;;
      --marionette-port)
        MARIONETTE_PORT="${2:-}"
        [[ "$MARIONETTE_PORT" =~ ^[0-9]+$ ]] && [[ "$MARIONETTE_PORT" -gt 0 ]] && [[ "$MARIONETTE_PORT" -le 65535 ]] || fail "--marionette-port requires a TCP port number from 1 to 65535"
        shift 2
        ;;
      --upload-file)
        UPLOAD_FILE="${2:-}"
        [[ -n "$UPLOAD_FILE" ]] || fail "--upload-file requires a value"
        shift 2
        ;;
      --file-input-selector)
        FILE_INPUT_SELECTOR="${2:-}"
        [[ -n "$FILE_INPUT_SELECTOR" ]] || fail "--file-input-selector requires a value"
        shift 2
        ;;
      --navigate-url)
        NAVIGATE_URL="${2:-}"
        [[ -n "$NAVIGATE_URL" ]] || fail "--navigate-url requires a value"
        shift 2
        ;;
      --helper-output)
        HELPER_OUTPUT="${2:-}"
        [[ -n "$HELPER_OUTPUT" ]] || fail "--helper-output requires a value"
        shift 2
        ;;
      --no-git-add-artifacts)
        GIT_ADD_ARTIFACTS="false"
        shift
        ;;
      --keep-runtime)
        KEEP_RUNTIME="true"
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        fail "unknown argument: $1"
        ;;
    esac
  done

  [[ -n "$COMMAND" ]] || COMMAND="inspect"
  [[ "$BROWSER_STARTUP_GRACE" =~ ^[0-9]+$ ]] || fail "TENDRIL_HEADLESS_BROWSER_STARTUP_GRACE must be a non-negative integer number of seconds"
  if [[ -n "$MARIONETTE_PORT" ]]; then
    [[ "$MARIONETTE_PORT" =~ ^[0-9]+$ ]] && [[ "$MARIONETTE_PORT" -gt 0 ]] && [[ "$MARIONETTE_PORT" -le 65535 ]] || fail "TENDRIL_HEADLESS_MARIONETTE_PORT must be a TCP port number from 1 to 65535"
  fi
}

runtime_dir() {
  printf '%s/%s' "${BASE_DIR%/}" "$NAME"
}

state_file() {
  printf '%s/state.env' "$(runtime_dir)"
}

load_state() {
  local file
  file="$(state_file)"
  if [[ -f "$file" ]]; then
    # shellcheck disable=SC1090
    source "$file"
    return 0
  fi
  return 1
}

pid_alive() {
  local pid="${1:-}"
  [[ -n "$pid" ]] && kill -0 "$pid" >/dev/null 2>&1
}

state_alive() {
  load_state || return 1
  pid_alive "${TENDRIL_HEADLESS_XVFB_PID:-}"
}

ensure_name_safe() {
  [[ "$NAME" =~ ^[A-Za-z0-9_.-]+$ ]] || fail "--name may only contain ASCII letters, digits, dot, underscore, or dash"
}

require_programs() {
  have Xvfb || fail "Xvfb is required; enter the Nix dev shell or install xorg-server/Xvfb"
  have python3 || fail "python3 is required for the smoke helper and JSON extraction"
}

write_browser_smoke_page() {
  local dir="$1"
  cat >"$dir/browser-smoke.html" <<'EOF_HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Tendril Smoke Browser</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background: #172a45;
      color: #f8fbff;
      font: 32px/1.4 system-ui, sans-serif;
    }
    main {
      width: min(1200px, calc(100vw - 160px));
      padding: 56px;
      border: 4px solid #67e8f9;
      border-radius: 32px;
      background: #071222;
    }
    h1 { margin: 0 0 24px; font-size: 56px; }
    label { display: grid; gap: 16px; font-weight: 700; }
    input {
      width: 100%;
      box-sizing: border-box;
      padding: 24px 28px;
      border: 3px solid #facc15;
      border-radius: 18px;
      background: #fff8dc;
      color: #111827;
      font: 36px/1.2 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }
    #status {
      margin: 28px 0 0;
      padding: 20px 24px;
      border-left: 10px solid #22c55e;
      background: rgba(34, 197, 94, 0.16);
      font-weight: 800;
    }
  </style>
</head>
<body>
  <main>
    <h1>Tendril Smoke Browser</h1>
    <label for="smoke-input">
      Browser-controlled input target
      <input id="smoke-input" autofocus value="" placeholder="waiting for Tendril input">
    </label>
    <p id="status">Waiting for Tendril browser control.</p>
  </main>
  <script>
    const input = document.getElementById('smoke-input');
    const status = document.getElementById('status');
    function mark(prefix) {
      status.textContent = `${prefix}: ${input.value || '(empty)'}`;
      document.body.dataset.tendrilSmokeValue = input.value;
    }
    input.addEventListener('input', () => mark('Tendril input visible in browser'));
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        mark('Tendril browser control confirmed');
      }
    });
    window.addEventListener('load', () => {
      input.focus();
      setTimeout(() => input.focus(), 250);
      setTimeout(() => input.focus(), 1000);
    });
  </script>
</body>
</html>
EOF_HTML
}

browser_env() {
  local display="$1" dir="$2"
  shift 2
  env -u WAYLAND_DISPLAY \
    DISPLAY="$display" \
    XDG_SESSION_TYPE=x11 \
    GDK_BACKEND=x11 \
    QT_QPA_PLATFORM=xcb \
    MOZ_ENABLE_WAYLAND=0 \
    ELECTRON_OZONE_PLATFORM_HINT=x11 \
    HOME="$dir/home" \
    XDG_CONFIG_HOME="$dir/config" \
    XDG_CACHE_HOME="$dir/cache" \
    XDG_STATE_HOME="$dir/state" \
    TMPDIR="$dir/tmp" \
    "$@"
}

resolve_browser_candidate() {
  local candidate="$1"
  if command -v "$candidate" >/dev/null 2>&1; then
    command -v "$candidate"
    return 0
  fi
  if [[ -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
    return 0
  fi
  return 1
}

choose_browsers() {
  if [[ -n "$BROWSER_BIN" ]]; then
    resolve_browser_candidate "$BROWSER_BIN" || fail "browser not found or not executable: $BROWSER_BIN"
    return 0
  fi

  local candidate resolved seen
  seen=""
  for candidate in chromium chromium-browser google-chrome-stable google-chrome firefox firefox-esr; do
    if resolved="$(resolve_browser_candidate "$candidate")"; then
      case " $seen " in
        *" $resolved "*) ;;
        *)
          printf '%s\n' "$resolved"
          seen="$seen $resolved"
          ;;
      esac
    fi
  done

  [[ -n "$seen" ]] || fail "no supported browser found on PATH; install chromium/google-chrome/firefox or pass --browser"
}

choose_window_manager() {
  local candidate
  for candidate in openbox fluxbox matchbox-window-manager twm; do
    if have "$candidate"; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  printf ''
}

choose_terminal() {
  local candidate
  for candidate in xterm uxterm; do
    if have "$candidate"; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  printf ''
}

safe_log_name() {
  printf '%s' "$(basename "$1")" | tr -c 'A-Za-z0-9_.-' '_'
}

job_running() {
  local target="$1" job_pid
  for job_pid in $(jobs -pr); do
    if [[ "$job_pid" == "$target" ]]; then
      return 0
    fi
  done
  return 1
}

wait_for_child_status() {
  local pid="$1" status
  set +e
  wait "$pid"
  status="$?"
  set -e
  printf '%s' "$status"
}

diagnose_browser_log() {
  local log_file="$1"
  [[ -f "$log_file" ]] || return 0
  if grep -Eq 'Trace/breakpoint trap|crashpad|Crashpad|scaling_cur_freq|No usable sandbox|SUID sandbox|namespace|zygote|core dumped' "$log_file"; then
    log "browser diagnostic excerpts from $log_file:"
    grep -E 'Trace/breakpoint trap|crashpad|Crashpad|scaling_cur_freq|No usable sandbox|SUID sandbox|namespace|zygote|core dumped' "$log_file" \
      | tail -20 \
      | while IFS= read -r line; do log "  $line"; done
  fi
}

browser_startup_alive() {
  local pid="$1" browser="$2" log_file="$3" attempts attempt status
  attempts=$((BROWSER_STARTUP_GRACE * 10))
  if [[ "$attempts" -le 0 ]]; then
    attempts=1
  fi
  for attempt in $(seq 1 "$attempts"); do
    if ! job_running "$pid"; then
      status="$(wait_for_child_status "$pid")"
      log "browser exited during startup: $browser (status $status); see $log_file"
      diagnose_browser_log "$log_file"
      return 1
    fi
    sleep 0.1
  done
  return 0
}

launch_browser() {
  local display="$1" dir="$2" browser="$3" smoke_url="$4" attempt="$5" browser_base failed_log
  browser_base="$(safe_log_name "$browser")"
  log "starting browser: $browser"
  case "$(basename "$browser")" in
    firefox|firefox-esr)
      local firefox_profile
      local -a firefox_args
      firefox_profile="$dir/browser-profile/firefox"
      mkdir -p "$firefox_profile"
      firefox_args=(
        --no-remote
        --new-instance
        --profile "$firefox_profile"
        --width "$WIDTH"
        --height "$HEIGHT"
        "$smoke_url"
      )
      if [[ -n "${MARIONETTE_PORT:-}" ]]; then
        cat >"$firefox_profile/user.js" <<EOF_FIREFOX_PREFS
user_pref("marionette.enabled", true);
user_pref("marionette.port", ${MARIONETTE_PORT});
EOF_FIREFOX_PREFS
        firefox_args=(-marionette "${firefox_args[@]}")
      fi
      MOZ_MARIONETTE=1 browser_env "$display" "$dir" "$browser" \
        "${firefox_args[@]}" \
        >"$dir/logs/browser.log" 2>&1 &
      browser_pid="$!"
      ;;
    *)
      local -a chromium_args=(
        --ozone-platform=x11
        --user-data-dir="$dir/browser-profile/chromium"
        --no-first-run
        --no-default-browser-check
        --disable-background-networking
        --disable-breakpad
        --disable-component-update
        --disable-crash-reporter
        --disable-crashpad
        --disable-dev-shm-usage
        --disable-extensions
        --disable-features=Crashpad
        --disable-gpu
        --disable-notifications
        --disable-software-rasterizer
        --disable-sync
        --enable-automation
        --password-store=basic
        --test-type
        --window-size="${WIDTH},${HEIGHT}"
        --start-maximized
        --app="$smoke_url"
      )
      if [[ "$CHROMIUM_SANDBOX" != "true" ]]; then
        chromium_args=(--no-sandbox --disable-setuid-sandbox "${chromium_args[@]}")
      fi
      if [[ "$BROWSER_RENDERER_LIMIT" -gt 0 ]]; then
        chromium_args=(--renderer-process-limit="$BROWSER_RENDERER_LIMIT" "${chromium_args[@]}")
      fi
      browser_env "$display" "$dir" "$browser" \
        "${chromium_args[@]}" \
        >"$dir/logs/browser.log" 2>&1 &
      browser_pid="$!"
      ;;
  esac

  if browser_startup_alive "$browser_pid" "$browser" "$dir/logs/browser.log"; then
    return 0
  fi

  failed_log="$dir/logs/browser-${attempt}-${browser_base}.failed.log"
  cp "$dir/logs/browser.log" "$failed_log" >/dev/null 2>&1 || true
  log "preserved failed browser attempt log: $failed_log"
  browser_pid=""
  return 1
}

launch_browser_candidates() {
  local display="$1" dir="$2" smoke_url="$3" candidate attempt
  shift 3
  attempt=1
  browser=""
  browser_pid=""
  for candidate in "$@"; do
    if launch_browser "$display" "$dir" "$candidate" "$smoke_url" "$attempt"; then
      browser="$candidate"
      return 0
    fi
    attempt=$((attempt + 1))
    if [[ -n "$BROWSER_BIN" ]]; then
      break
    fi
    log "trying next browser candidate after $candidate failed"
  done
  return 1
}

choose_display() {
  if [[ -n "$DISPLAY_NUMBER" ]]; then
    printf '%s' "$DISPLAY_NUMBER"
    return 0
  fi

  local number
  for number in $(seq 90 199); do
    if [[ ! -e "/tmp/.X${number}-lock" && ! -S "/tmp/.X11-unix/X${number}" ]]; then
      printf '%s' "$number"
      return 0
    fi
  done
  fail "could not find a free X display in :90..:199"
}

write_state() {
  local dir="$1" display="$2" xvfb_pid="$3" wm_pid="$4" browser_pid="$5" terminal_pid="$6" browser="$7" wm="$8" terminal="$9" marionette_port="${10:-}"
  cat > "$(state_file)" <<EOF_STATE
export TENDRIL_HEADLESS_NAME=$(quote "$NAME")
export TENDRIL_HEADLESS_RUNTIME_DIR=$(quote "$dir")
export TENDRIL_HEADLESS_WIDTH=$(quote "$WIDTH")
export TENDRIL_HEADLESS_HEIGHT=$(quote "$HEIGHT")
export TENDRIL_HEADLESS_DEPTH=$(quote "$DEPTH")
export TENDRIL_HEADLESS_BROWSER=$(quote "$browser")
export TENDRIL_HEADLESS_WINDOW_MANAGER=$(quote "$wm")
export TENDRIL_HEADLESS_TERMINAL=$(quote "$terminal")
export TENDRIL_HEADLESS_MARIONETTE_PORT=$(quote "$marionette_port")
export TENDRIL_HEADLESS_XVFB_PID=$(quote "$xvfb_pid")
export TENDRIL_HEADLESS_WM_PID=$(quote "$wm_pid")
export TENDRIL_HEADLESS_BROWSER_PID=$(quote "$browser_pid")
export TENDRIL_HEADLESS_TERMINAL_PID=$(quote "$terminal_pid")
export DISPLAY=$(quote "$display")
export XDG_SESSION_TYPE=x11
EOF_STATE
}

print_exports() {
  load_state || fail "environment '$NAME' is not running"
  cat <<EOF_EXPORTS
export DISPLAY=$(quote "$DISPLAY")
export XDG_SESSION_TYPE=x11
export TENDRIL_HEADLESS_NAME=$(quote "$TENDRIL_HEADLESS_NAME")
export TENDRIL_HEADLESS_RUNTIME_DIR=$(quote "$TENDRIL_HEADLESS_RUNTIME_DIR")
export TENDRIL_HEADLESS_WIDTH=$(quote "$TENDRIL_HEADLESS_WIDTH")
export TENDRIL_HEADLESS_HEIGHT=$(quote "$TENDRIL_HEADLESS_HEIGHT")
export TENDRIL_HEADLESS_MARIONETTE_PORT=$(quote "${TENDRIL_HEADLESS_MARIONETTE_PORT:-}")
EOF_EXPORTS
}

wait_for_x() {
  local display="$1"
  local attempt
  for attempt in $(seq 1 80); do
    if DISPLAY="$display" xdpyinfo >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_env() {
  ensure_name_safe
  require_programs

  if state_alive; then
    log "reusing environment '$NAME' on DISPLAY=${DISPLAY}"
    print_exports
    return 0
  fi

  local dir display_number display browser wm terminal xvfb_pid wm_pid browser_pid terminal_pid browser_list
  local -a browser_candidates
  dir="$(runtime_dir)"
  rm -rf "$dir"
  mkdir -p "$dir/logs" "$dir/browser-profile" "$dir/home" "$dir/tmp" "$dir/config" "$dir/cache" "$dir/state"

  display_number="$(choose_display)"
  display=":${display_number}"
  MARIONETTE_PORT="$(choose_marionette_port "$display_number")"
  browser_list="$(choose_browsers)"
  mapfile -t browser_candidates <<<"$browser_list"
  wm="$(choose_window_manager)"
  terminal="$(choose_terminal)"

  log "starting Xvfb ${display} at ${WIDTH}x${HEIGHT}x${DEPTH}"
  Xvfb "$display" \
    -screen 0 "${WIDTH}x${HEIGHT}x${DEPTH}" \
    -nolisten tcp \
    -noreset \
    +extension RANDR \
    +extension XTEST \
    >"$dir/logs/xvfb.log" 2>&1 &
  xvfb_pid="$!"

  if ! wait_for_x "$display"; then
    kill "$xvfb_pid" >/dev/null 2>&1 || true
    fail "Xvfb did not become ready; see $dir/logs/xvfb.log"
  fi

  DISPLAY="$display" xsetroot -solid '#101820' >/dev/null 2>&1 || true

  wm_pid=""
  if [[ -n "$wm" ]]; then
    log "starting window manager: $wm"
    DISPLAY="$display" HOME="$dir/home" "$wm" >"$dir/logs/window-manager.log" 2>&1 &
    wm_pid="$!"
    sleep 0.5
  else
    log "no lightweight window manager found; continuing with bare Xvfb"
  fi

  terminal_pid=""
  if [[ -n "$terminal" ]]; then
    DISPLAY="$display" HOME="$dir/home" "$terminal" \
      -geometry 100x24+40+80 \
      -title "Tendril Headless Shell" \
      -e sh -lc 'printf "Tendril headless shell ready.\\n"; exec sh' \
      >"$dir/logs/terminal.log" 2>&1 &
    terminal_pid="$!"
  fi

  local smoke_page smoke_url
  write_browser_smoke_page "$dir"
  smoke_page="$dir/browser-smoke.html"
  smoke_url="file://${smoke_page}"

  browser_pid=""
  if ! launch_browser_candidates "$display" "$dir" "$smoke_url" "${browser_candidates[@]}"; then
    log "all browser candidates failed before Tendril discovery; logs are under $dir/logs"
    for pid in "$terminal_pid" "$wm_pid" "$xvfb_pid"; do
      if pid_alive "$pid"; then
        kill "$pid" >/dev/null 2>&1 || true
      fi
    done
    fail "could not keep a supported browser alive for the headless environment"
  fi

  write_state "$dir" "$display" "$xvfb_pid" "$wm_pid" "$browser_pid" "$terminal_pid" "$browser" "$wm" "$terminal" "$MARIONETTE_PORT"
  sleep 1
  log "started environment '$NAME'; logs are under $dir/logs"
  print_exports
}

stop_env() {
  ensure_name_safe
  local dir pids pid
  if ! load_state; then
    log "environment '$NAME' is not running"
    return 0
  fi
  dir="${TENDRIL_HEADLESS_RUNTIME_DIR:-$(runtime_dir)}"
  pids=(
    "${TENDRIL_HEADLESS_BROWSER_PID:-}"
    "${TENDRIL_HEADLESS_TERMINAL_PID:-}"
    "${TENDRIL_HEADLESS_WM_PID:-}"
    "${TENDRIL_HEADLESS_XVFB_PID:-}"
  )
  for pid in "${pids[@]}"; do
    if pid_alive "$pid"; then
      kill "$pid" >/dev/null 2>&1 || true
    fi
  done
  sleep 0.4
  for pid in "${pids[@]}"; do
    if pid_alive "$pid"; then
      kill -9 "$pid" >/dev/null 2>&1 || true
    fi
  done

  if [[ "$KEEP_RUNTIME" == "true" ]]; then
    rm -f "$(state_file)"
    log "stopped environment '$NAME' and kept $dir"
  else
    rm -rf "$dir"
    log "stopped environment '$NAME' and removed $dir"
  fi
}

inspect_env() {
  ensure_name_safe
  if ! load_state; then
    printf 'name: %s\nstatus: stopped\nruntime_dir: %s\n' "$NAME" "$(runtime_dir)"
    return 0
  fi

  printf 'name: %s\n' "${TENDRIL_HEADLESS_NAME:-$NAME}"
  printf 'status: %s\n' "$(pid_alive "${TENDRIL_HEADLESS_XVFB_PID:-}" && printf running || printf stale)"
  printf 'display: %s\n' "${DISPLAY:-}"
  printf 'resolution: %sx%sx%s\n' "${TENDRIL_HEADLESS_WIDTH:-}" "${TENDRIL_HEADLESS_HEIGHT:-}" "${TENDRIL_HEADLESS_DEPTH:-}"
  printf 'runtime_dir: %s\n' "${TENDRIL_HEADLESS_RUNTIME_DIR:-}"
  printf 'browser: %s pid=%s alive=%s\n' "${TENDRIL_HEADLESS_BROWSER:-}" "${TENDRIL_HEADLESS_BROWSER_PID:-}" "$(pid_alive "${TENDRIL_HEADLESS_BROWSER_PID:-}" && printf true || printf false)"
  printf 'marionette_port: %s\n' "${TENDRIL_HEADLESS_MARIONETTE_PORT:-}"
  printf 'window_manager: %s pid=%s alive=%s\n' "${TENDRIL_HEADLESS_WINDOW_MANAGER:-}" "${TENDRIL_HEADLESS_WM_PID:-}" "$(pid_alive "${TENDRIL_HEADLESS_WM_PID:-}" && printf true || printf false)"
  printf 'terminal: %s pid=%s alive=%s\n' "${TENDRIL_HEADLESS_TERMINAL:-}" "${TENDRIL_HEADLESS_TERMINAL_PID:-}" "$(pid_alive "${TENDRIL_HEADLESS_TERMINAL_PID:-}" && printf true || printf false)"
}

run_tendril() {
  local -a tendril_argv
  # Allows --tendril-bin './target/debug/tendril' as well as
  # --tendril-bin 'nix run .#tendril --' for checkout-fresh execution.
  read -r -a tendril_argv <<<"$TENDRIL_BIN"
  DISPLAY="$DISPLAY" XDG_SESSION_TYPE=x11 "${tendril_argv[@]}" "$@"
}

resolve_artifact_dir() {
  python3 -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$ARTIFACT_DIR"
}

ensure_artifact_dir_safe() {
  local dir="$1"
  case "$dir" in
    /tmp|/tmp/*|/var/tmp|/var/tmp/*|/run|/run/*)
      fail "smoke artifacts must not live in tmp/runtime storage; use --artifact-dir summaries/<agent-id> so Tendril captures are git-trackable"
      ;;
  esac
}

git_add_artifacts() {
  local dir="$1"
  if [[ "$GIT_ADD_ARTIFACTS" != "true" ]]; then
    return 0
  fi
  if git rev-parse --show-toplevel >/dev/null 2>&1; then
    git add -- "$dir" >/dev/null 2>&1 || log "could not git-add artifact directory $dir"
  fi
}

preserve_runtime_logs() {
  local runtime="$1" artifact_dir="$2" target
  [[ -d "$runtime/logs" ]] || return 0
  [[ -n "$artifact_dir" ]] || return 0
  target="$artifact_dir/runtime-logs"
  mkdir -p "$target"
  cp -a "$runtime/logs/." "$target/" >/dev/null 2>&1 || true
  git_add_artifacts "$target"
  log "preserved runtime logs under $target"
}

wait_for_targets() {
  local list_json attempt browser_pid
  browser_pid="${TENDRIL_HEADLESS_BROWSER_PID:-}"
  for attempt in $(seq 1 120); do
    if list_json="$(run_tendril --json list 2>/dev/null)"; then
      if python3 -c '
import json, sys
width=int(sys.argv[1])
height=int(sys.argv[2])
browser_pid=sys.argv[3]
payload=json.load(sys.stdin)
targets=payload.get("data",{}).get("targets",[])
has_display=any(t.get("kind")=="display" and t.get("bounds",{}).get("width")==width and t.get("bounds",{}).get("height")==height for t in targets)

def is_browser(target):
    if target.get("kind") != "window":
        return False
    if browser_pid and str(target.get("process_id") or "") == browser_pid:
        return True
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    return any(token in haystack for token in ("chrom", "chrome", "firefox", "browser", "tendril smoke browser"))

has_browser=any(is_browser(t) for t in targets)
sys.exit(0 if has_display and has_browser else 1)
' "$WIDTH" "$HEIGHT" "$browser_pid" <<<"$list_json"; then
        printf '%s' "$list_json"
        return 0
      fi
    fi
    sleep 0.25
  done
  return 1
}

run_smoke() {
  ensure_name_safe
  local tendril_program
  tendril_program="${TENDRIL_BIN%% *}"
  [[ -x "$tendril_program" || "$(command -v "$tendril_program" 2>/dev/null || true)" ]] || fail "Tendril binary not found: $TENDRIL_BIN; pass --tendril-bin ./target/debug/tendril or --tendril-bin 'nix run .#tendril --'"

  local artifact_dir
  artifact_dir="$(resolve_artifact_dir)"
  ensure_artifact_dir_safe "$artifact_dir"
  mkdir -p "$artifact_dir"

  local started_here="false"
  if ! state_alive; then
    start_env >/dev/null
    started_here="true"
  else
    log "using existing environment '$NAME' on DISPLAY=${DISPLAY}"
  fi
  trap 'if [[ "${started_here:-false}" == "true" ]]; then stop_env; fi' EXIT

  local dir list_json display_id window_id xterm_window_id capture_json run_json xterm_run_json browser_capture_json xterm_manifest_note
  dir="${TENDRIL_HEADLESS_RUNTIME_DIR:-$(runtime_dir)}"
  xterm_manifest_note="xterm_shift_insert_run=skipped"

  log "waiting for Tendril to see a ${WIDTH}x${HEIGHT} display and browser window"
  if ! list_json="$(wait_for_targets)"; then
    diagnose_browser_log "$dir/logs/browser.log"
    preserve_runtime_logs "$dir" "$artifact_dir"
    if [[ "$started_here" == "true" ]]; then
      stop_env || true
      trap - EXIT
    fi
    fail "Tendril did not discover expected headless browser targets; runtime logs were preserved under $artifact_dir/runtime-logs (rerun with --keep-runtime to also keep $dir)"
  fi
  printf '%s\n' "$list_json" >"$artifact_dir/${NAME}-list.json"

  display_id="$(python3 -c '
import json, sys
width=int(sys.argv[1])
height=int(sys.argv[2])
payload=json.load(sys.stdin)
for target in payload["data"]["targets"]:
    bounds=target.get("bounds", {})
    if target.get("kind") == "display" and bounds.get("width") == width and bounds.get("height") == height:
        print(target["id"])
        break
else:
    raise SystemExit(1)
' "$WIDTH" "$HEIGHT" <<<"$list_json")"

  window_id="$(python3 -c '
import json, sys
browser_pid=sys.argv[1]
payload=json.load(sys.stdin)

def is_browser(target):
    if target.get("kind") != "window":
        return False
    if browser_pid and str(target.get("process_id") or "") == browser_pid:
        return True
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    return any(token in haystack for token in ("chrom", "chrome", "firefox", "browser", "tendril smoke browser"))

for target in payload["data"]["targets"]:
    if is_browser(target):
        print(target["id"])
        break
else:
    windows=[t for t in payload["data"]["targets"] if t.get("kind") == "window"]
    summaries=[{"id": t.get("id"), "app_name": t.get("app_name"), "name": t.get("name"), "title": t.get("title")} for t in windows]
    raise SystemExit(f"no browser window discovered; discovered windows: {summaries!r}")
' "${TENDRIL_HEADLESS_BROWSER_PID:-}" <<<"$list_json")"

  log "capturing display $display_id into $artifact_dir"
  capture_json="$(run_tendril --json --display "$display_id" capture --max-width "$WIDTH" --max-height "$HEIGHT" -o "$artifact_dir/${NAME}-display.png")"
  printf '%s\n' "$capture_json" >"$artifact_dir/${NAME}-capture.json"
  python3 -c '
import json, sys
width=int(sys.argv[1])
height=int(sys.argv[2])
payload=json.load(sys.stdin)
assert payload["status"] == "success"
assert payload["data"]["output_bounds"]["width"] == width
assert payload["data"]["output_bounds"]["height"] == height
' "$WIDTH" "$HEIGHT" <<<"$capture_json"

  log "running browser-visible input against window $window_id"
  run_json="$(run_tendril --json --window "$window_id" run 'send("tendril browser control confirmed"),Return,wait(500ms)')"
  printf '%s\n' "$run_json" >"$artifact_dir/${NAME}-run.json"
  python3 -c '
import json, sys
payload=json.load(sys.stdin)
assert payload["status"] == "success"
assert payload["data"]["action_count"] >= 2
assert payload["data"]["focus_required"] is True
' <<<"$run_json"

  xterm_window_id="$(python3 -c '
import json, sys
terminal_pid=sys.argv[1]
payload=json.load(sys.stdin)
for target in payload["data"].get("targets", []):
    if target.get("kind") != "window":
        continue
    if terminal_pid and str(target.get("process_id") or "") == terminal_pid:
        print(target["id"])
        break
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if any(token in haystack for token in ("xterm", "uxterm", "tendril headless shell")):
        print(target["id"])
        break
' "${TENDRIL_HEADLESS_TERMINAL_PID:-}" <<<"$list_json" || true)"

  if [[ -n "$xterm_window_id" ]]; then
    log "running XTerm Shift+Insert paste shortcut smoke against window $xterm_window_id"
    xterm_run_json="$(run_tendril --json --window "$xterm_window_id" run 'hold(shift),Insert,release(shift),wait(100ms)')"
    printf '%s\n' "$xterm_run_json" >"$artifact_dir/${NAME}-xterm-shift-insert-run.json"
    python3 -c '
import json, sys
payload=json.load(sys.stdin)
assert payload["status"] == "success"
assert payload["data"]["action_count"] >= 3
' <<<"$xterm_run_json"
    xterm_manifest_note="xterm_shift_insert_window=$xterm_window_id
xterm_shift_insert_run=$artifact_dir/${NAME}-xterm-shift-insert-run.json"
  else
    log "skipping XTerm Shift+Insert paste shortcut smoke because no XTerm window target was discovered"
    xterm_manifest_note="xterm_shift_insert_run=skipped (no XTerm window target discovered)"
  fi

  log "capturing controlled browser window $window_id into $artifact_dir"
  browser_capture_json="$(run_tendril --json --window "$window_id" capture --max-width "$WIDTH" --max-height "$HEIGHT" -o "$artifact_dir/${NAME}-browser-after.png")"
  printf '%s\n' "$browser_capture_json" >"$artifact_dir/${NAME}-browser-after-capture.json"
  python3 -c '
import json, sys
payload=json.load(sys.stdin)
assert payload["status"] == "success"
assert payload["data"].get("output_bounds", {}).get("width", 0) > 0
assert payload["data"].get("output_bounds", {}).get("height", 0) > 0
' <<<"$browser_capture_json"

  cat >"$artifact_dir/${NAME}-manifest.txt" <<EOF_MANIFEST
Tendril headless browser smoke passed.
name=$NAME
display=$DISPLAY
browser=${TENDRIL_HEADLESS_BROWSER:-}
browser_window=$window_id
${xterm_manifest_note}
resolution=${WIDTH}x${HEIGHT}x${DEPTH}
runtime_dir=$dir
artifacts=$artifact_dir
EOF_MANIFEST
  git_add_artifacts "$artifact_dir"

  log "smoke passed; artifacts: $artifact_dir/${NAME}-{list,capture,run,browser-after-capture}.json $artifact_dir/${NAME}-{display,browser-after}.png"
  if [[ "$started_here" == "true" ]]; then
    stop_env
    trap - EXIT
  fi
}

write_file_upload_smoke_page() {
  local dir="$1"
  cat >"$dir/file-upload-task.html" <<'EOF_HTML'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Tendril File Upload Task</title>
  <style>
    :root { color-scheme: dark; }
    body {
      margin: 0;
      min-height: 100vh;
      box-sizing: border-box;
      padding: 64px 84px;
      background: #111827;
      color: #f9fafb;
      font: 28px/1.45 system-ui, sans-serif;
    }
    main { max-width: 1280px; }
    h1 { margin: 0 0 20px; font-size: 52px; }
    .upload-box {
      margin-top: 32px;
      padding: 32px;
      border: 4px solid #38bdf8;
      border-radius: 24px;
      background: #0f172a;
    }
    input[type=file] {
      display: block;
      width: 900px;
      max-width: 100%;
      padding: 18px;
      border: 3px solid #facc15;
      border-radius: 14px;
      background: #fff8dc;
      color: #111827;
      font: 30px/1.2 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }
    #status {
      margin-top: 32px;
      white-space: pre-wrap;
      padding: 24px;
      min-height: 180px;
      border-left: 12px solid #22c55e;
      background: rgba(34, 197, 94, 0.16);
      font: 28px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }
  </style>
</head>
<body>
  <main>
    <h1>Tendril File Upload Task</h1>
    <p>Choose the proof file. The page prints the selected file name and contents.</p>
    <section class="upload-box">
      <label for="upload-input">Upload proof file</label>
      <input id="upload-input" type="file">
    </section>
    <pre id="status">Waiting for upload.</pre>
  </main>
  <script>
    const input = document.getElementById('upload-input');
    const status = document.getElementById('status');
    input.addEventListener('change', async () => {
      if (!input.files.length) {
        status.textContent = 'No file selected.';
        return;
      }
      const file = input.files[0];
      const text = await file.text();
      status.textContent = `Uploaded file confirmed\nname=${file.name}\ncontents=${text}`;
      document.body.dataset.uploadConfirmed = text;
    });
  </script>
</body>
</html>
EOF_HTML
}

run_firefox_upload_helper() {
  ensure_name_safe
  state_alive || fail "environment '$NAME' is not running; start it first or run file-upload-smoke"
  local browser_base port upload_abs output_target
  browser_base="$(basename "${TENDRIL_HEADLESS_BROWSER:-}")"
  case "$browser_base" in
    firefox|firefox-esr|.firefox-wrapper|firefox-bin) ;;
    *) fail "firefox-upload requires a Firefox headless environment, got browser=${TENDRIL_HEADLESS_BROWSER:-unknown}; start with --browser firefox" ;;
  esac
  port="${TENDRIL_HEADLESS_MARIONETTE_PORT:-}"
  [[ -n "$port" ]] || fail "running environment did not record a Marionette port; reset it with this updated helper"
  [[ -n "$UPLOAD_FILE" ]] || fail "firefox-upload requires --upload-file <path>"
  [[ -f "$UPLOAD_FILE" ]] || fail "upload file does not exist: $UPLOAD_FILE"
  upload_abs="$(abspath "$UPLOAD_FILE")"

  output_target="/dev/stdout"
  if [[ -n "$HELPER_OUTPUT" ]]; then
    mkdir -p "$(dirname "$HELPER_OUTPUT")"
    output_target="$HELPER_OUTPUT"
  fi

  python3 - "$port" "$FILE_INPUT_SELECTOR" "$upload_abs" "$NAVIGATE_URL" >"$output_target" <<'PY'
import json
import os
import socket
import sys
import time

port = int(sys.argv[1])
selector = sys.argv[2]
upload_file = sys.argv[3]
navigate_url = sys.argv[4]

if not os.path.isfile(upload_file):
    raise SystemExit(f"upload file not found: {upload_file}")

class Marionette:
    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        self.sock.settimeout(20)
        self.next_id = 0
        self.hello = self.recv()

    def close(self):
        self.sock.close()

    def recv(self):
        length_bytes = bytearray()
        while True:
            chunk = self.sock.recv(1)
            if not chunk:
                raise EOFError("Marionette closed while reading frame length")
            if chunk == b":":
                break
            length_bytes.extend(chunk)
        length = int(length_bytes.decode("ascii"))
        body = bytearray()
        while len(body) < length:
            chunk = self.sock.recv(length - len(body))
            if not chunk:
                raise EOFError("Marionette closed while reading frame body")
            body.extend(chunk)
        return json.loads(body.decode("utf-8"))

    def send_raw(self, payload):
        encoded = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.sock.sendall(str(len(encoded)).encode("ascii") + b":" + encoded)

    def command(self, name, params):
        self.next_id += 1
        message_id = self.next_id
        self.send_raw([0, message_id, name, params])
        response = self.recv()
        if not (isinstance(response, list) and len(response) == 4 and response[0] == 1 and response[1] == message_id):
            raise RuntimeError(f"unexpected Marionette response to {name}: {response!r}")
        error = response[2]
        if error is not None:
            raise RuntimeError(f"Marionette {name} failed: {error!r}")
        return response[3]

client = Marionette(port)
try:
    session = client.command("WebDriver:NewSession", {})
    if navigate_url:
        client.command("WebDriver:Navigate", {"url": navigate_url})
        time.sleep(0.5)
    found = client.command("WebDriver:FindElement", {"using": "css selector", "value": selector})
    element_value = found.get("value", found)
    element_id = (
        element_value.get("element-6066-11e4-a52e-4f735466cecf")
        or element_value.get("ELEMENT")
    )
    if not element_id:
        raise RuntimeError(f"could not extract element id from {found!r}")
    client.command(
        "WebDriver:ElementSendKeys",
        {"id": element_id, "text": upload_file, "value": list(upload_file)},
    )
    time.sleep(0.75)
    state = client.command(
        "WebDriver:ExecuteScript",
        {
            "script": """
const input = document.querySelector(arguments[0]);
return {
  title: document.title,
  url: location.href,
  selector: arguments[0],
  fileCount: input && input.files ? input.files.length : null,
  fileName: input && input.files && input.files.length ? input.files[0].name : null,
  bodyText: document.body ? document.body.innerText : null,
  uploadConfirmed: document.body ? document.body.dataset.uploadConfirmed || null : null
};
""",
            "args": [selector],
            "newSandbox": True,
            "sandbox": "default",
            "line": 1,
            "filename": "tendril-headless-firefox-upload",
        },
    )
    value = state.get("value", state)
    result = {
        "status": "success",
        "helper": "tendril-headless firefox-upload",
        "transport": "firefox-marionette",
        "marionette": {
            "port": port,
            "hello": client.hello,
            "sessionId": session.get("sessionId"),
        },
        "request": {
            "selector": selector,
            "upload_file": upload_file,
            "navigate_url": navigate_url or None,
        },
        "page": value,
    }
    print(json.dumps(result, indent=2, sort_keys=True))
finally:
    # Do not send WebDriver:DeleteSession here: for this direct Marionette
    # attachment it can close Firefox before Tendril captures the verified page.
    client.close()
PY
}

run_firefox_navigate_helper() {
  ensure_name_safe
  state_alive || fail "environment '$NAME' is not running"
  local browser_base port output_target
  browser_base="$(basename "${TENDRIL_HEADLESS_BROWSER:-}")"
  case "$browser_base" in
    firefox|firefox-esr|.firefox-wrapper|firefox-bin) ;;
    *) fail "Firefox Marionette navigation requires a Firefox headless environment, got browser=${TENDRIL_HEADLESS_BROWSER:-unknown}" ;;
  esac
  port="${TENDRIL_HEADLESS_MARIONETTE_PORT:-}"
  [[ -n "$port" ]] || fail "running environment did not record a Marionette port"
  [[ -n "$NAVIGATE_URL" ]] || fail "Firefox Marionette navigation requires NAVIGATE_URL"
  output_target="/dev/stdout"
  if [[ -n "$HELPER_OUTPUT" ]]; then
    mkdir -p "$(dirname "$HELPER_OUTPUT")"
    output_target="$HELPER_OUTPUT"
  fi
  python3 - "$port" "$NAVIGATE_URL" >"$output_target" <<'PY'
import json
import socket
import sys
import time

port = int(sys.argv[1])
url = sys.argv[2]

def recv(sock):
    length_bytes = bytearray()
    while True:
        chunk = sock.recv(1)
        if not chunk:
            raise EOFError("Marionette closed while reading frame length")
        if chunk == b":":
            break
        length_bytes.extend(chunk)
    length = int(length_bytes.decode("ascii"))
    body = bytearray()
    while len(body) < length:
        chunk = sock.recv(length - len(body))
        if not chunk:
            raise EOFError("Marionette closed while reading frame body")
        body.extend(chunk)
    return json.loads(body.decode("utf-8"))

def send(sock, payload):
    encoded = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sock.sendall(str(len(encoded)).encode("ascii") + b":" + encoded)

def command(sock, message_id, name, params):
    send(sock, [0, message_id, name, params])
    response = recv(sock)
    if response[2] is not None:
        raise RuntimeError(f"Marionette {name} failed: {response[2]!r}")
    return response[3]

sock = socket.create_connection(("127.0.0.1", port), timeout=5)
sock.settimeout(20)
try:
    hello = recv(sock)
    session = command(sock, 1, "WebDriver:NewSession", {})
    command(sock, 2, "WebDriver:Navigate", {"url": url})
    time.sleep(0.75)
    page = command(sock, 3, "WebDriver:ExecuteScript", {
        "script": "return { title: document.title, url: location.href, bodyText: document.body ? document.body.innerText : null };",
        "args": [],
        "newSandbox": True,
        "sandbox": "default",
        "line": 1,
        "filename": "tendril-headless-firefox-navigate",
    })
    print(json.dumps({
        "status": "success",
        "helper": "tendril-headless firefox-marionette-navigate",
        "transport": "firefox-marionette",
        "marionette": {"port": port, "hello": hello, "sessionId": session.get("sessionId")},
        "request": {"navigate_url": url},
        "page": page.get("value", page),
    }, indent=2, sort_keys=True))
finally:
    sock.close()
PY
}

run_file_upload_smoke() {
  ensure_name_safe
  if [[ -z "$BROWSER_BIN" ]]; then
    BROWSER_BIN="firefox"
  fi
  local tendril_program
  tendril_program="${TENDRIL_BIN%% *}"
  [[ -x "$tendril_program" || "$(command -v "$tendril_program" 2>/dev/null || true)" ]] || fail "Tendril binary not found: $TENDRIL_BIN; pass --tendril-bin ./target/debug/tendril or --tendril-bin 'nix run .#tendril --'"

  local artifact_dir
  artifact_dir="$(resolve_artifact_dir)"
  ensure_artifact_dir_safe "$artifact_dir"
  mkdir -p "$artifact_dir/upload-source"

  cat >"$artifact_dir/upload-source/upload-proof.txt" <<'EOF_PROOF'
tendril-file-upload-control-ok
EOF_PROOF
  write_file_upload_smoke_page "$artifact_dir"
  local upload_url
  upload_url="file://$artifact_dir/file-upload-task.html"

  local started_here="false"
  if ! state_alive; then
    start_env >/dev/null
    started_here="true"
  else
    log "using existing environment '$NAME' on DISPLAY=${DISPLAY}"
  fi
  trap 'if [[ "${started_here:-false}" == "true" ]]; then stop_env; fi' EXIT

  local dir list_json display_id window_id before_capture click_run after_click_list after_click_capture dismiss_run helper_json after_capture
  dir="${TENDRIL_HEADLESS_RUNTIME_DIR:-$(runtime_dir)}"

  log "waiting for Tendril to see a ${WIDTH}x${HEIGHT} Firefox window"
  if ! list_json="$(wait_for_targets)"; then
    diagnose_browser_log "$dir/logs/browser.log"
    preserve_runtime_logs "$dir" "$artifact_dir"
    fail "Tendril did not discover expected headless Firefox targets"
  fi
  printf '%s\n' "$list_json" >"$artifact_dir/${NAME}-fileupload-list-initial.json"

  display_id="$(python3 -c '
import json, sys
width=int(sys.argv[1]); height=int(sys.argv[2]); payload=json.load(sys.stdin)
for target in payload["data"]["targets"]:
    bounds=target.get("bounds", {})
    if target.get("kind") == "display" and bounds.get("width") == width and bounds.get("height") == height:
        print(target["id"]); break
else:
    raise SystemExit(1)
' "$WIDTH" "$HEIGHT" <<<"$list_json")"

  window_id="$(python3 -c '
import json, sys
browser_pid=sys.argv[1]
payload=json.load(sys.stdin)
for target in payload["data"]["targets"]:
    haystack=" ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if target.get("kind") == "window" and ((browser_pid and str(target.get("process_id") or "") == browser_pid) or "firefox" in haystack):
        print(target["id"]); break
else:
    raise SystemExit("no Firefox window found")
' "${TENDRIL_HEADLESS_BROWSER_PID:-}" <<<"$list_json")"

  log "navigating Firefox to file-upload smoke page through Marionette preflight"
  NAVIGATE_URL="$upload_url" \
    HELPER_OUTPUT="$artifact_dir/${NAME}-fileupload-marionette-navigate.json" \
    run_firefox_navigate_helper

  log "capturing upload form before native chooser click"
  before_capture="$(run_tendril --json --window "$window_id" capture --max-width "$WIDTH" --max-height "$HEIGHT" -o "$artifact_dir/${NAME}-fileupload-before.png")"
  printf '%s\n' "$before_capture" >"$artifact_dir/${NAME}-fileupload-before-capture.json"

  log "clicking the native Firefox file input Browse control with Tendril"
  click_run="$(run_tendril --json --window "$window_id" run 'lclick(180,305),wait(1000ms)')"
  printf '%s\n' "$click_run" >"$artifact_dir/${NAME}-fileupload-native-click-run.json"

  after_click_list="$(run_tendril --json list 2>"$artifact_dir/${NAME}-fileupload-list-after-native-click.err")"
  printf '%s\n' "$after_click_list" >"$artifact_dir/${NAME}-fileupload-list-after-native-click.json"
  after_click_capture="$(run_tendril --json --display "$display_id" capture --max-width "$WIDTH" --max-height "$HEIGHT" -o "$artifact_dir/${NAME}-fileupload-after-native-click-display.png")"
  printf '%s\n' "$after_click_capture" >"$artifact_dir/${NAME}-fileupload-after-native-click-display-capture.json"

  python3 - "$after_click_list" <<'PY'
import json
import sys
payload = json.loads(sys.argv[1])
windows = [t for t in payload.get("data", {}).get("targets", []) if t.get("kind") == "window"]
dialogs = []
for target in windows:
    haystack = " ".join(str(target.get(k) or "") for k in ("name", "title", "app_name")).lower()
    if any(token in haystack for token in ("open file", "file chooser", "choose file", "picker")):
        dialogs.append(target)
if dialogs:
    raise SystemExit(f"unexpected discoverable native chooser target(s): {dialogs!r}")
PY

  log "dismissing any browser-modal chooser state before helper upload"
  dismiss_run="$(run_tendril --json --window "$window_id" run 'Escape,wait(500ms)' || true)"
  printf '%s\n' "$dismiss_run" >"$artifact_dir/${NAME}-fileupload-dismiss-native-run.json"

  log "uploading proof file through Firefox Marionette helper"
  UPLOAD_FILE="$artifact_dir/upload-source/upload-proof.txt" \
    FILE_INPUT_SELECTOR='input[type="file"]' \
    NAVIGATE_URL="$upload_url" \
    HELPER_OUTPUT="$artifact_dir/${NAME}-fileupload-helper.json" \
    run_firefox_upload_helper
  helper_json="$(cat "$artifact_dir/${NAME}-fileupload-helper.json")"
  python3 -c '
import json, sys
payload=json.loads(sys.argv[1])
assert payload["status"] == "success"
page = payload["page"]
assert page["fileCount"] == 1
assert page["fileName"] == "upload-proof.txt"
body = page.get("bodyText") or ""
assert "Uploaded file confirmed" in body
assert "tendril-file-upload-control-ok" in body
' "$helper_json"

  log "capturing verified uploaded page through Tendril"
  after_capture="$(run_tendril --json --window "$window_id" capture --max-width "$WIDTH" --max-height "$HEIGHT" -o "$artifact_dir/${NAME}-fileupload-after-helper.png")"
  printf '%s\n' "$after_capture" >"$artifact_dir/${NAME}-fileupload-after-helper-capture.json"

  cat >"$artifact_dir/${NAME}-fileupload-manifest.txt" <<EOF_MANIFEST
Tendril headless Firefox file-upload smoke passed.
name=$NAME
display=$DISPLAY
browser=${TENDRIL_HEADLESS_BROWSER:-}
browser_window=$window_id
marionette_port=${TENDRIL_HEADLESS_MARIONETTE_PORT:-}
resolution=${WIDTH}x${HEIGHT}x${DEPTH}
runtime_dir=$dir
upload_url=$upload_url
proof_file=$artifact_dir/upload-source/upload-proof.txt
native_chooser_result=no separate chooser target discovered after Tendril click; upload completed via Firefox Marionette helper.
artifacts=$artifact_dir
EOF_MANIFEST
  git_add_artifacts "$artifact_dir"

  log "file-upload smoke passed; artifacts are under $artifact_dir"
  if [[ "$started_here" == "true" ]]; then
    stop_env
    trap - EXIT
  fi
}

parse_args "$@"
case "$COMMAND" in
  start) start_env ;;
  env) print_exports ;;
  inspect) inspect_env ;;
  reset) stop_env; start_env ;;
  stop) stop_env ;;
  smoke) run_smoke ;;
  firefox-upload) run_firefox_upload_helper ;;
  file-upload-smoke) run_file_upload_smoke ;;
  *) fail "unsupported command: $COMMAND" ;;
esac
