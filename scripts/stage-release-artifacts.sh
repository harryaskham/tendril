#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

# shellcheck source=./release-lib.sh
source "${script_dir}/release-lib.sh"

assert_release_tag_matches_version "${1:-${GITHUB_REF_NAME:-}}"

version="$(workspace_version)"
target="$(release_target)"
asset_name="$(release_asset_name "${version}" "${target}")"
checksum_name="$(release_checksum_name "${version}" "${target}")"
dist_dir="${TENDRIL_RELEASE_DIST:-${repo_root}/dist/release}"
out_link="${dist_dir}/result"

rm -rf "${dist_dir}"
mkdir -p "${dist_dir}"

(
  cd "${repo_root}"
  nix build .#releaseArtifact --out-link "${out_link}"
)

cp "${out_link}/${asset_name}" "${dist_dir}/${asset_name}"
cp "${out_link}/${checksum_name}" "${dist_dir}/${checksum_name}"
cp "${out_link}/release-manifest.json" "${dist_dir}/release-manifest.json"
rm -f "${out_link}"

printf 'staged %s\n' "${dist_dir}/${asset_name}"
printf 'staged %s\n' "${dist_dir}/${checksum_name}"
printf 'staged %s\n' "${dist_dir}/release-manifest.json"
