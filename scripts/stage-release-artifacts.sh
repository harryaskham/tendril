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
stage_root="${dist_dir}/stage/tendril-${version}-${target}"

rm -rf "${dist_dir}"
mkdir -p "${stage_root}"

(
  cd "${repo_root}"
  # Release assets must contain the raw Cargo binary. `nix build .#tendril`
  # intentionally creates a runtime wrapper and hidden `.tendril-wrapped`
  # binary, which is valid inside the Nix store but not portable to updater
  # installs under ~/.local/bin.
  nix develop --command cargo build --release --locked -p tendril --bin tendril

  if [[ "${target}" == *-darwin ]]; then
    bin="target/release/tendril"
    while IFS= read -r ref; do
      [[ -n "${ref}" ]] && install_name_tool -change "${ref}" /usr/lib/libiconv.2.dylib "${bin}"
    done < <(otool -L "${bin}" | grep -oE '/nix/store/[^ ]*libiconv[^ ]*\.dylib' || true)
    remaining="$(otool -L "${bin}" | grep /nix/store || true)"
    if [[ -n "${remaining}" ]]; then
      echo "Darwin release binary still references the Nix store:" >&2
      echo "${remaining}" >&2
      exit 1
    fi
  fi

  cp "target/release/tendril" "${stage_root}/tendril"
)
chmod +x "${stage_root}/tendril"

tar -C "${dist_dir}/stage" -czf "${dist_dir}/${asset_name}" "tendril-${version}-${target}"
if command -v sha256sum >/dev/null 2>&1; then
  hash_line="$(sha256sum "${dist_dir}/${asset_name}")"
else
  hash_line="$(shasum -a 256 "${dist_dir}/${asset_name}")"
fi
hash="${hash_line%% *}"
printf '%s  %s\n' "${hash}" "${asset_name}" > "${dist_dir}/${checksum_name}"
rm -rf "${dist_dir}/stage"

printf 'staged %s\n' "${dist_dir}/${asset_name}"
printf 'staged %s\n' "${dist_dir}/${checksum_name}"
