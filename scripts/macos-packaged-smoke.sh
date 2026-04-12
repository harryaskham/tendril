#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

# shellcheck source=./release-lib.sh
source "${script_dir}/release-lib.sh"

dist_dir="${TENDRIL_RELEASE_DIST:-${repo_root}/dist/macos-smoke}"
tag="${1:-$(release_tag)}"
version="$(workspace_version)"
target="$(release_target)"
asset_name="$(release_asset_name "${version}" "${target}")"
asset_path="${dist_dir}/${asset_name}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf 'macOS packaged smoke coverage only runs on Darwin hosts\n' >&2
  exit 1
fi

if [[ ! -f "${asset_path}" ]]; then
  TENDRIL_RELEASE_DIST="${dist_dir}" "${script_dir}/stage-release-artifacts.sh" "${tag}"
fi

workdir="$(mktemp -d "${TMPDIR:-/tmp}/tendril-macos-smoke.XXXXXX")"
trap 'rm -rf "${workdir}"' EXIT

tar -xzf "${asset_path}" -C "${workdir}"
binary="${workdir}/tendril"
config_dir="${workdir}/config"
stdout_path="${workdir}/list.stdout"
stderr_path="${workdir}/list.stderr"

mkdir -p "${config_dir}"
chmod +x "${binary}"

set +e
TENDRIL_CONFIG_DIR="${config_dir}" "${binary}" --json list >"${stdout_path}" 2>"${stderr_path}"
status=$?
set -e

python3 - "$stdout_path" "$stderr_path" "$status" "$asset_path" <<'PY'
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
    r"failed to execute `swift`",
    r"`swift` exited with status",
    r"swift: command not found",
    r"No such file or directory[^\n]*swift",
]
for pattern in runtime_dependency_patterns:
    if re.search(pattern, combined, re.IGNORECASE):
        raise SystemExit(
            f"packaged macOS smoke failed because Tendril still appears to rely on swift at runtime; matched {pattern!r}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )

try:
    envelope = json.loads(stdout)
except json.JSONDecodeError as error:
    raise SystemExit(
        f"packaged macOS smoke expected JSON stdout from tendril list but failed to decode it: {error}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    )

meta = envelope.get("meta") or {}
if meta.get("command") != "list":
    raise SystemExit(f"expected tendril list envelope meta.command=list, got: {meta!r}")

status_value = envelope.get("status")
if status == 0:
    if status_value != "success":
        raise SystemExit(f"expected success envelope for zero exit status, got: {envelope!r}")
    data = envelope.get("data") or {}
    adapter = data.get("adapter") or {}
    if adapter.get("platform") != "mac_os":
        raise SystemExit(f"expected macOS adapter metadata in successful list output, got: {adapter!r}")
    if adapter.get("stateless") is not True:
        raise SystemExit(f"expected packaged list flow to remain stateless, got adapter metadata: {adapter!r}")
    targets = data.get("targets")
    if not isinstance(targets, list):
        raise SystemExit(f"expected successful list output to contain a targets array, got: {targets!r}")
    permissions = data.get("permissions")
    if not isinstance(permissions, list) or not permissions:
        raise SystemExit(f"expected successful list output to contain explicit permissions, got: {permissions!r}")

    guided_permissions = [
        permission
        for permission in permissions
        if permission.get("permission") in {"screen_capture", "accessibility"}
        and permission.get("state") in {"unknown", "denied"}
    ]
    if not guided_permissions:
        raise SystemExit(
            "expected successful macOS list output to carry capture/input permission guidance when permissions are not fully known"
        )
    for permission in guided_permissions:
        if not str(permission.get("summary", "")).strip():
            raise SystemExit(f"permission guidance missing summary: {permission!r}")
        if not str(permission.get("suggested_action", "")).strip():
            raise SystemExit(f"permission guidance missing suggested_action: {permission!r}")
else:
    if status_value != "error":
        raise SystemExit(f"expected error envelope for non-zero exit status, got: {envelope!r}")
    error = envelope.get("error") or {}
    if error.get("code") != "missing_permission":
        raise SystemExit(
            f"expected packaged macOS list failure to be structured missing_permission guidance, got: {error!r}"
        )
    details = error.get("details") or {}
    if details.get("permission") != "screen_capture":
        raise SystemExit(f"expected screen_capture permission guidance, got: {details!r}")
    suggested_action = str(details.get("suggested_action", "")).strip()
    if not suggested_action:
        raise SystemExit(f"missing permission guidance lacked suggested_action: {error!r}")
    if "screen" not in suggested_action.lower():
        raise SystemExit(f"expected screen-recording guidance in suggested_action, got: {suggested_action!r}")

print(f"Verified packaged macOS tendril list smoke coverage via {asset_path}")
PY
