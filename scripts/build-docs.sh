#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOK_DIR="$ROOT_DIR/target/book"
RUSTDOC_DIR="$ROOT_DIR/target/doc"

cd "$ROOT_DIR"

printf '==> building mdBook content\n'
mdbook build docs

printf '==> generating workspace rustdoc\n'
cargo doc --workspace --no-deps

printf '==> assembling Pages artifact\n'
rm -rf "$BOOK_DIR/api"
mkdir -p "$BOOK_DIR/api"
cp -R "$RUSTDOC_DIR/." "$BOOK_DIR/api/"

printf '==> docs ready at %s\n' "$BOOK_DIR/index.html"
