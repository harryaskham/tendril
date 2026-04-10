#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

workspace_version() {
  local manifest="${repo_root}/Cargo.toml"

  if command -v nix >/dev/null 2>&1; then
    nix eval --impure --raw --expr "let manifest = builtins.fromTOML (builtins.readFile ${manifest}); in manifest.workspace.package.version" 2>/dev/null && return 0
  fi

  sed -n '/^\[workspace.package\]/,/^\[/{ s/^version = "\([^"]*\)"$/\1/p; }' "${manifest}" | head -n 1
}

release_tag() {
  printf 'v%s\n' "$(workspace_version)"
}

release_target() {
  local os="${1:-$(uname -s)}"
  local arch="${2:-$(uname -m)}"

  case "${os}:${arch}" in
    Linux:x86_64)
      printf 'x86_64-linux\n'
      ;;
    Linux:aarch64|Linux:arm64)
      printf 'aarch64-linux\n'
      ;;
    Darwin:arm64|Darwin:aarch64)
      printf 'aarch64-darwin\n'
      ;;
    Darwin:x86_64)
      printf 'x86_64-darwin\n'
      ;;
    *)
      printf 'unsupported Tendril release target: %s:%s\n' "${os}" "${arch}" >&2
      return 1
      ;;
  esac
}

release_asset_name() {
  local version="${1:-$(workspace_version)}"
  local target="${2:-$(release_target)}"

  printf 'tendril-%s-%s.tar.gz\n' "${version}" "${target}"
}

release_checksum_name() {
  local version="${1:-$(workspace_version)}"
  local target="${2:-$(release_target)}"

  printf 'tendril-%s-%s.sha256\n' "${version}" "${target}"
}

assert_release_tag_matches_version() {
  local supplied_tag="${1:-${GITHUB_REF_NAME:-}}"
  local expected_tag
  expected_tag="$(release_tag)"

  if [[ -z "${supplied_tag}" ]]; then
    return 0
  fi

  if [[ "${supplied_tag}" != "${expected_tag}" ]]; then
    printf 'release tag %s does not match Cargo workspace version tag %s\n' "${supplied_tag}" "${expected_tag}" >&2
    return 1
  fi
}

usage() {
  cat <<'EOF'
Usage: scripts/release-lib.sh <command> [args]

Commands:
  version                 Print the Cargo workspace SemVer.
  tag                     Print the canonical git tag name for the workspace version.
  target [os] [arch]      Print the canonical release platform suffix.
  asset-name [v] [t]      Print the tarball asset name for the version/target.
  checksum-name [v] [t]   Print the checksum asset name for the version/target.
EOF
}

main() {
  local command="${1:-}"

  case "${command}" in
    version)
      workspace_version
      ;;
    tag)
      release_tag
      ;;
    target)
      release_target "${2:-}" "${3:-}"
      ;;
    asset-name)
      release_asset_name "${2:-}" "${3:-}"
      ;;
    checksum-name)
      release_checksum_name "${2:-}" "${3:-}"
      ;;
    "")
      usage
      ;;
    *)
      printf 'unknown command: %s\n\n' "${command}" >&2
      usage >&2
      return 1
      ;;
  esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
