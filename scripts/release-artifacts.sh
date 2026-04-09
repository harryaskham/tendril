#!/usr/bin/env bash
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$ROOT"

tag=${1:-${GITHUB_REF_NAME:-}}
if [[ -z "$tag" ]]; then
  echo "usage: $0 <tag>" >&2
  exit 1
fi

if [[ "$tag" != v* ]]; then
  echo "expected a SemVer tag prefixed with v, got: $tag" >&2
  exit 1
fi

version=${tag#v}
workspace_version=$(
  awk '
    $0 == "[workspace.package]" { in_workspace = 1; next }
    /^\[/ && $0 != "[workspace.package]" { in_workspace = 0 }
    in_workspace && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
)

if [[ -z "$workspace_version" ]]; then
  echo "could not determine workspace version from Cargo.toml" >&2
  exit 1
fi

if [[ "$workspace_version" != "$version" ]]; then
  echo "tag version $version does not match workspace version $workspace_version" >&2
  exit 1
fi

system=$(nix eval --impure --raw --expr 'builtins.currentSystem')
archive_ref=HEAD
if git rev-parse --verify --quiet "$tag" >/dev/null; then
  archive_ref=$tag
fi

rm -rf dist
mkdir -p "dist/stage/tendril-${version}-${system}"

nix build .#tendril --out-link dist/result-tendril --print-build-logs

cp -L dist/result-tendril/bin/tendril "dist/stage/tendril-${version}-${system}/tendril"
cp README.md CHANGELOG.md "dist/stage/tendril-${version}-${system}/"

tar -C dist/stage -czf "dist/tendril-${version}-${system}.tar.gz" "tendril-${version}-${system}"
git archive --format=tar.gz --prefix="tendril-${version}/" -o "dist/tendril-${version}-source.tar.gz" "$archive_ref"

for artifact in dist/*.tar.gz; do
  checksum=$(sha256sum "$artifact" | awk '{ print $1 }')
  printf '%s  %s\n' "$checksum" "$(basename "$artifact")" > "${artifact}.sha256"
done

printf 'Prepared release artifacts for %s (%s):\n' "$version" "$system"
find dist -maxdepth 1 -type f | sort
