#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

# shellcheck source=./release-lib.sh
source "${script_dir}/release-lib.sh"

dist_dir="${TENDRIL_RELEASE_DIST:-${repo_root}/dist/linux-x11-smoke}"
tag="${1:-$(release_tag)}"
version="$(workspace_version)"
target="$(release_target)"
asset_name="$(release_asset_name "${version}" "${target}")"
asset_path="${dist_dir}/${asset_name}"

if [[ "$(uname -s)" != "Linux" ]]; then
  printf 'Linux/X11 packaged smoke coverage only runs on Linux hosts\n' >&2
  exit 1
fi

if [[ -z "${DISPLAY:-}" ]]; then
  printf 'Linux/X11 packaged smoke coverage requires DISPLAY to be set\n' >&2
  exit 1
fi

if [[ "${XDG_SESSION_TYPE:-x11}" != "x11" ]]; then
  printf 'Linux/X11 packaged smoke coverage requires an active X11 session (XDG_SESSION_TYPE=x11)\n' >&2
  exit 1
fi

if [[ ! -f "${asset_path}" ]]; then
  TENDRIL_RELEASE_DIST="${dist_dir}" "${script_dir}/stage-release-artifacts.sh" "${tag}"
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/tendril-linux-x11-smoke.XXXXXX")"
trap 'rm -rf "${workdir}"' EXIT

tar -xzf "${asset_path}" -C "${workdir}"
binary="${workdir}/tendril"
config_dir="${workdir}/config"
list_stdout="${workdir}/list.stdout"
list_stderr="${workdir}/list.stderr"
capture_stdout="${workdir}/capture.stdout"
capture_stderr="${workdir}/capture.stderr"
run_stdout="${workdir}/run.stdout"
run_stderr="${workdir}/run.stderr"

mkdir -p "${config_dir}"
chmod +x "${binary}"

set +e
TENDRIL_CONFIG_DIR="${config_dir}" "${binary}" --json list >"${list_stdout}" 2>"${list_stderr}"
list_status=$?
set -e

capture_target="$(${PYTHON:-python3} - "$list_stdout" "$list_stderr" "$list_status" "$asset_path" <<'PY'
import json
import pathlib
import re
import sys

stdout_path = pathlib.Path(sys.argv[1])
stderr_path = pathlib.Path(sys.argv[2])
status = int(sys.argv[3])
asset_path = sys.argv[4]
stdout = stdout_path.read_text()
stderr = stderr_path.read_text()
combined = f"{stdout}\n{stderr}"

runtime_dependency_patterns = [
    r"failed to execute `xrandr`",
    r"failed to execute `xprop`",
    r"failed to execute `xwininfo`",
    r"xrandr: command not found",
    r"xprop: command not found",
    r"xwininfo: command not found",
    r"No such file or directory[^\n]*(xrandr|xprop|xwininfo)",
]
for pattern in runtime_dependency_patterns:
    if re.search(pattern, combined, re.IGNORECASE):
        raise SystemExit(
            f"packaged Linux/X11 list smoke failed because Tendril still appears to rely on external X11 helper tools; matched {pattern!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )

try:
    envelope = json.loads(stdout)
except json.JSONDecodeError as error:
    raise SystemExit(
        f"packaged Linux/X11 smoke expected JSON stdout from tendril list but failed to decode it: {error}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )

if status != 0:
    raise SystemExit(
        f"packaged Linux/X11 list smoke expected success inside an active X11 session, got exit status {status}\nenvelope:\n{json.dumps(envelope, indent=2)}\nstderr:\n{stderr}"
    )

if envelope.get("status") != "success":
    raise SystemExit(f"expected success envelope from packaged list smoke, got: {envelope!r}")

meta = envelope.get("meta") or {}
if meta.get("command") != "list":
    raise SystemExit(f"expected tendril list envelope meta.command=list, got: {meta!r}")

data = envelope.get("data") or {}
adapter = data.get("adapter") or {}
if adapter.get("platform") != "linux":
    raise SystemExit(f"expected Linux adapter metadata, got: {adapter!r}")
if adapter.get("session") != "x11":
    raise SystemExit(f"expected X11 session metadata, got: {adapter!r}")
if adapter.get("stateless") is not True:
    raise SystemExit(f"expected packaged X11 list flow to remain stateless, got: {adapter!r}")

targets = data.get("targets")
if not isinstance(targets, list) or not targets:
    raise SystemExit(f"expected packaged X11 list output to contain targets, got: {targets!r}")

permissions = data.get("permissions")
if not isinstance(permissions, list) or not permissions:
    raise SystemExit(f"expected packaged X11 list output to contain permissions, got: {permissions!r}")

for permission in permissions:
    if not str(permission.get("summary", "")).strip():
        raise SystemExit(f"permission summary missing from packaged X11 list output: {permission!r}")

for target in targets:
    if target.get("kind") == "display":
        print(target.get("id", ""))
        break
else:
    raise SystemExit(f"expected at least one display target in packaged X11 list output, got: {targets!r}")
PY
)"

set +e
TENDRIL_CONFIG_DIR="${config_dir}" "${binary}" --display "${capture_target}" --json capture >"${capture_stdout}" 2>"${capture_stderr}"
capture_status=$?
set -e

${PYTHON:-python3} - "$capture_stdout" "$capture_stderr" "$capture_status" "$asset_path" <<'PY'
import json
import pathlib
import re
import sys

stdout_path = pathlib.Path(sys.argv[1])
stderr_path = pathlib.Path(sys.argv[2])
status = int(sys.argv[3])
asset_path = sys.argv[4]
stdout = stdout_path.read_text()
stderr = stderr_path.read_text()
combined = f"{stdout}\n{stderr}"

runtime_dependency_patterns = [
    r"failed to execute `import`",
    r"import: command not found",
    r"No such file or directory[^\n]*import",
]
for pattern in runtime_dependency_patterns:
    if re.search(pattern, combined, re.IGNORECASE):
        raise SystemExit(
            f"packaged Linux/X11 capture smoke failed because Tendril still appears to rely on ImageMagick import; matched {pattern!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )

try:
    envelope = json.loads(stdout)
except json.JSONDecodeError as error:
    raise SystemExit(
        f"packaged Linux/X11 smoke expected JSON stdout from tendril capture but failed to decode it: {error}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )

if status != 0 or envelope.get("status") != "success":
    raise SystemExit(
        f"packaged Linux/X11 capture smoke expected success for the discovered display target, got exit status {status}\nenvelope:\n{json.dumps(envelope, indent=2)}\nstderr:\n{stderr}"
    )

meta = envelope.get("meta") or {}
if meta.get("command") != "capture":
    raise SystemExit(f"expected tendril capture envelope meta.command=capture, got: {meta!r}")

data = envelope.get("data") or {}
adapter = data.get("adapter") or {}
if adapter.get("platform") != "linux" or adapter.get("session") != "x11":
    raise SystemExit(f"expected Linux/X11 adapter metadata in packaged capture output, got: {adapter!r}")
if not str(data.get("image_base64", "")).strip():
    raise SystemExit(f"expected packaged capture output to include image data, got: {data!r}")
if data.get("media_type") not in {"image/png", "image/jpeg"}:
    raise SystemExit(f"unexpected capture media_type in packaged X11 smoke output: {data.get('media_type')!r}")
if not isinstance(data.get("output_bounds"), dict):
    raise SystemExit(f"expected capture output_bounds metadata in packaged X11 smoke output, got: {data!r}")

print(f"Verified packaged Linux/X11 tendril list+capture smoke coverage via {asset_path}")
PY

if [[ -n "${TENDRIL_X11_SMOKE_RUN_TARGET:-}" && -n "${TENDRIL_X11_SMOKE_RUN_SEQUENCE:-}" ]]; then
  set +e
  TENDRIL_CONFIG_DIR="${config_dir}" \
    "${binary}" \
    --window "${TENDRIL_X11_SMOKE_RUN_TARGET}" \
    --json run "${TENDRIL_X11_SMOKE_RUN_SEQUENCE}" >"${run_stdout}" 2>"${run_stderr}"
  run_status=$?
  set -e

  ${PYTHON:-python3} - "$run_stdout" "$run_stderr" "$run_status" <<'PY'
import json
import pathlib
import re
import sys

stdout = pathlib.Path(sys.argv[1]).read_text()
stderr = pathlib.Path(sys.argv[2]).read_text()
status = int(sys.argv[3])
combined = f"{stdout}\n{stderr}"

runtime_dependency_patterns = [
    r"failed to execute `xdotool`",
    r"xdotool: command not found",
    r"No such file or directory[^\n]*xdotool",
]
for pattern in runtime_dependency_patterns:
    if re.search(pattern, combined, re.IGNORECASE):
        raise SystemExit(
            f"packaged Linux/X11 run smoke failed because Tendril still appears to rely on xdotool; matched {pattern!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )

envelope = json.loads(stdout)
if status != 0 or envelope.get("status") != "success":
    raise SystemExit(
        f"packaged Linux/X11 run smoke expected success, got exit status {status}\nenvelope:\n{json.dumps(envelope, indent=2)}\nstderr:\n{stderr}"
    )

print("Verified packaged Linux/X11 run smoke coverage without xdotool")
PY
else
  printf 'Skipped packaged Linux/X11 run smoke: set TENDRIL_X11_SMOKE_RUN_TARGET and TENDRIL_X11_SMOKE_RUN_SEQUENCE to opt into real input injection validation.\n'
fi
