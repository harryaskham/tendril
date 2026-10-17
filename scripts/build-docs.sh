#!/usr/bin/env bash
# Assemble the documentation site for GitHub Pages into target/site:
#   - site/ (hand-written pages; validated first by scripts/check-site.py)
#   - workspace rustdoc under api/
# Run via `nix develop --command ./scripts/build-docs.sh` (.github/workflows/pages.yaml
# runs it on the shared self-hosted publisher). Edit pages in site/, then run
# `python3 scripts/site/build.py` and `python3 scripts/check-site.py`.
# docs/src/ keeps the long-form operator guides as repository documentation.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/target/site"
RUSTDOC_DIR="$ROOT_DIR/target/doc"

cd "$ROOT_DIR"

printf '==> validating site/\n'
python3 scripts/check-site.py

printf '==> copying site/\n'
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
cp -R site/. "$OUT_DIR/"

printf '==> generating workspace rustdoc\n'
cargo doc --workspace --no-deps

mkdir -p "$OUT_DIR/api"
cp -R "$RUSTDOC_DIR/." "$OUT_DIR/api/"

printf '==> site ready at %s\n' "$OUT_DIR/index.html"
