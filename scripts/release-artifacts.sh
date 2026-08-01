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

TENDRIL_RELEASE_DIST="${ROOT}/dist" "${ROOT}/scripts/stage-release-artifacts.sh" "$tag"

binary_archive="$(release_asset_name "$version" "$system")"
binary_checksum="$(release_checksum_name "$version" "$system")"
source_archive="tendril-${version}-source.tar.gz"
source_checksum="tendril-${version}-source.sha256"

git archive --format=tar.gz --prefix="tendril-${version}/" -o "dist/${source_archive}" "$archive_ref"
if command -v sha256sum >/dev/null 2>&1; then
  source_hash_line="$(sha256sum "dist/${source_archive}")"
else
  source_hash_line="$(shasum -a 256 "dist/${source_archive}")"
fi
source_sum="${source_hash_line%% *}"
printf '%s  %s\n' "$source_sum" "$source_archive" > "dist/${source_checksum}"

cat > dist/release-manifest.json <<EOF
{"project":"tendril","version":"${version}","semver":"${version}","tag":"${tag}","system":"${system}","updater":"updatable-cli","asset_strategy":"TendrilStyle","artifacts":[{"name":"${binary_archive}","kind":"archive","format":"tar.gz","scope":"binary"},{"name":"${binary_checksum}","kind":"checksum","format":"sha256","scope":"binary"},{"name":"${source_archive}","kind":"archive","format":"tar.gz","scope":"source"},{"name":"${source_checksum}","kind":"checksum","format":"sha256","scope":"source"}]}
EOF

printf 'Prepared portable Cargo release artifacts for %s (%s):\n' "$version" "$system"
find dist -maxdepth 1 -type f | sort
