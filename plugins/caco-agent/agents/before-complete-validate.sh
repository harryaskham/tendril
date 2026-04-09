#!/usr/bin/env bash
set -euo pipefail

CWD=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$CWD"

if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
  echo "before-complete-validate: BLOCKED" >&2
  echo "  - uncommitted changes in working tree — commit or stash before completing" >&2
  exit 1
fi

./scripts/pre-merge.sh

echo "before-complete-validate: OK"
