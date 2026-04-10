#!/usr/bin/env bash
set -euo pipefail

ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
cd "$ROOT"

# shellcheck source=./release-lib.sh
source "${ROOT}/scripts/release-lib.sh"

tag=${1:-${GITHUB_REF_NAME:-}}
if [[ -z "$tag" ]]; then
  echo "usage: $0 <tag>" >&2
  exit 1
fi

assert_release_tag_matches_version "$tag"
version=${tag#v}
system="$(release_target)"
archive_ref=HEAD
if git rev-parse --verify --quiet "$tag" >/dev/null; then
  archive_ref=$tag
fi

binary_archive="$(release_asset_name "$version" "$system")"
binary_checksum="$(release_checksum_name "$version" "$system")"
source_archive="tendril-${version}-source.tar.gz"
source_checksum="tendril-${version}-source.sha256"

rm -rf dist
mkdir -p "dist/stage/tendril-${version}-${system}"

nix build .#tendril --out-link dist/result-tendril --print-build-logs

cp -L dist/result-tendril/bin/tendril "dist/stage/tendril-${version}-${system}/tendril"
cp README.md CHANGELOG.md LICENSE PROJECT_HEALTH.md "dist/stage/tendril-${version}-${system}/"

tar -C dist/stage -czf "dist/${binary_archive}" "tendril-${version}-${system}"
git archive --format=tar.gz --prefix="tendril-${version}/" -o "dist/${source_archive}" "$archive_ref"

binary_sum="$(sha256sum "dist/${binary_archive}" | cut -d' ' -f1)"
printf '%s  %s\n' "$binary_sum" "$binary_archive" > "dist/${binary_checksum}"

source_sum="$(sha256sum "dist/${source_archive}" | cut -d' ' -f1)"
printf '%s  %s\n' "$source_sum" "$source_archive" > "dist/${source_checksum}"

cat > dist/release-manifest.json <<EOF
{"project":"tendril","version":"${version}","semver":"${version}","tag":"${tag}","system":"${system}","artifacts":[{"name":"${binary_archive}","kind":"archive","format":"tar.gz","scope":"binary"},{"name":"${binary_checksum}","kind":"checksum","format":"sha256","scope":"binary"},{"name":"${source_archive}","kind":"archive","format":"tar.gz","scope":"source"},{"name":"${source_checksum}","kind":"checksum","format":"sha256","scope":"source"}]}
EOF

printf 'Prepared release artifacts for %s (%s):\n' "$version" "$system"
find dist -maxdepth 1 -type f | sort
