#!/usr/bin/env bash
set -euo pipefail

run() {
  printf '==> %s\n' "$*"
  "$@"
}

if command -v nix >/dev/null 2>&1; then
  run nix develop --command cargo fmt --all -- --check
  run nix develop --command cargo clippy --workspace --all-targets --all-features -- -D warnings
  run nix develop --command cargo test --workspace --all-features
  run nix flake check
else
  run cargo fmt --all -- --check
  run cargo clippy --workspace --all-targets --all-features -- -D warnings
  run cargo test --workspace --all-features
fi
