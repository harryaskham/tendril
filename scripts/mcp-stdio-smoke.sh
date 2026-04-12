#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

if [[ "${1:-}" == "--" ]]; then
  shift
fi

cd "${repo_root}"

python3 - "$@" <<'PY'
import json
import os
import subprocess
import sys
from typing import Any

cmd = sys.argv[1:] or ["nix", "run", ".#tendril", "--", "mcp", "stdio"]
requests = [
    {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {},
    },
    {
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {},
    },
    {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {},
    },
    {
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "list",
            "arguments": {},
        },
    },
]


def frame(message: dict[str, Any]) -> bytes:
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    return f"Content-Length: {len(body)}\r\n\r\n".encode("ascii") + body


def parse_frames(stream: bytes) -> list[dict[str, Any]]:
    responses: list[dict[str, Any]] = []
    cursor = 0
    while cursor < len(stream):
        header_end = stream.find(b"\r\n\r\n", cursor)
        if header_end == -1:
            raise SystemExit(
                f"framed MCP response was truncated after {cursor} bytes: {stream[cursor:]!r}"
            )
        header = stream[cursor:header_end].decode("utf-8")
        cursor = header_end + 4
        length = None
        for line in header.split("\r\n"):
            if line.lower().startswith("content-length:"):
                length = int(line.split(":", 1)[1].strip())
                break
        if length is None:
            raise SystemExit(f"missing Content-Length header in response header: {header!r}")
        body = stream[cursor : cursor + length]
        if len(body) != length:
            raise SystemExit(
                f"expected {length} response bytes but only received {len(body)}"
            )
        responses.append(json.loads(body))
        cursor += length
    return responses


payload = b"".join(frame(request) for request in requests)

proc = subprocess.Popen(
    cmd,
    cwd=os.getcwd(),
    env=os.environ.copy(),
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)

stdout, stderr = proc.communicate(payload)
if proc.returncode != 0:
    raise SystemExit(
        "external MCP smoke command failed with status "
        f"{proc.returncode}\nstdout:\n{stdout.decode('utf-8', errors='replace')}"
        f"\nstderr:\n{stderr.decode('utf-8', errors='replace')}"
    )

responses = parse_frames(stdout)
if len(responses) != 3:
    raise SystemExit(
        f"expected 3 MCP responses (initialize, tools/list, tools/call) but received {len(responses)}: {responses!r}"
    )

initialize, tools_list, tool_call = responses
server_info = ((initialize.get("result") or {}).get("serverInfo") or {})
if server_info.get("name") != "tendril":
    raise SystemExit(f"expected initialize serverInfo.name=tendril, got: {initialize!r}")

list_result = tools_list.get("result") or {}
tools = list_result.get("tools")
if not isinstance(tools, list):
    raise SystemExit(f"tools/list did not return a tools array: {tools_list!r}")

tool_names = [tool.get("name") for tool in tools]
if tool_names != ["list", "capture", "run"]:
    raise SystemExit(f"expected MCP tools [list, capture, run], got: {tool_names!r}")

by_name = {tool["name"]: tool for tool in tools}
list_schema = by_name["list"].get("inputSchema") or {}
capture_schema = by_name["capture"].get("inputSchema") or {}
run_schema = by_name["run"].get("inputSchema") or {}

if list_schema.get("type") != "object":
    raise SystemExit(f"expected list input schema to be an object, got: {list_schema!r}")

capture_properties = set((capture_schema.get("properties") or {}).keys())
expected_capture_properties = {
    "window",
    "display",
    "max_width",
    "max_height",
    "format",
    "compression",
}
if capture_properties != expected_capture_properties:
    raise SystemExit(
        "capture schema drifted from the published Tendril contract; "
        f"expected {sorted(expected_capture_properties)!r}, got {sorted(capture_properties)!r}"
    )

run_properties = set((run_schema.get("properties") or {}).keys())
expected_run_properties = {"window", "display", "input_definition"}
if run_properties != expected_run_properties:
    raise SystemExit(
        "run schema drifted from the published Tendril contract; "
        f"expected {sorted(expected_run_properties)!r}, got {sorted(run_properties)!r}"
    )

call_result = tool_call.get("result") or {}
structured = call_result.get("structuredContent") or {}
meta = structured.get("meta") or {}
if meta.get("command") != "list":
    raise SystemExit(f"expected tools/call(list) to return meta.command=list, got: {tool_call!r}")

status = structured.get("status")
if status == "success":
    data = structured.get("data") or {}
    if not isinstance(data.get("targets"), list):
        raise SystemExit(f"successful list call did not expose a targets array: {structured!r}")
elif status == "error":
    error = structured.get("error") or {}
    required_fields = ["category", "code", "message"]
    missing = [field for field in required_fields if not str(error.get(field, "")).strip()]
    if missing:
        raise SystemExit(
            f"error result from tools/call(list) was not a structured Tendril envelope; missing {missing!r}: {structured!r}"
        )
else:
    raise SystemExit(f"unexpected tools/call(list) envelope status: {structured!r}")

print(
    "Verified external MCP smoke for "
    + " ".join(cmd)
    + f" with tools {tool_names} and list status {status}."
)
PY
