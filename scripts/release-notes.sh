#!/usr/bin/env bash
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$ROOT"

tag=${1:-${GITHUB_REF_NAME:-}}
if [[ -z "$tag" ]]; then
  echo "usage: $0 <tag>" >&2
  exit 1
fi

version=${tag#v}
start_line="$({ grep -nE "^## \[(v)?${version//./\\.}\]" CHANGELOG.md || true; } | head -n 1 | cut -d: -f1)"

if [[ -z "$start_line" ]]; then
  echo "release notes for version $version were not found in CHANGELOG.md" >&2
  exit 1
fi

rest_start=$((start_line + 1))
next_heading_offset="$({ tail -n +"${rest_start}" CHANGELOG.md | grep -n '^## \[' || true; } | head -n 1 | cut -d: -f1)"

if [[ -n "$next_heading_offset" ]]; then
  end_line=$((start_line + next_heading_offset - 1))
else
  end_line='$'
fi

sed -n "${start_line},${end_line}p" CHANGELOG.md
