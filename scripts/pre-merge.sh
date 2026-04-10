#!/usr/bin/env bash
set -euo pipefail

run() {
  printf '==> %s\n' "$*"
  "$@"
}

if command -v nix >/dev/null 2>&1; then
  run nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).fmt
  run nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).clippy
  run nix build .#checks.$(nix eval --impure --raw --expr builtins.currentSystem).tests
  run nix flake check
else
  run cargo fmt --all -- --check
  run cargo clippy --workspace --all-targets --all-features -- -D warnings
  run cargo test --workspace --all-features
fi
