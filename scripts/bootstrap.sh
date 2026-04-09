#!/usr/bin/env bash
set -euo pipefail

./scripts/install-git-hooks.sh

direnv allow || true

if command -v direnv >/dev/null 2>&1; then
  eval "$(direnv export bash)"
fi
