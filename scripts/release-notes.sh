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

awk -v version="$version" '
  BEGIN {
    in_section = 0
    found = 0
  }
  $0 ~ "^## \\[" version "\\]" {
    in_section = 1
    found = 1
    print
    next
  }
  in_section && /^## \[/ {
    exit
  }
  in_section {
    print
  }
  END {
    if (!found) {
      printf("release notes for version %s were not found in CHANGELOG.md\n", version) > "/dev/stderr"
      exit 1
    }
  }
' CHANGELOG.md
