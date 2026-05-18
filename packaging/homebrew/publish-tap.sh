#!/usr/bin/env bash
set -euo pipefail

formula_path="${1:-dist/homebrew/browser-use-rs.rb}"
tap_repo="${HOMEBREW_TAP_REPOSITORY:-evalops/homebrew-tap}"
tap_branch="${HOMEBREW_TAP_BRANCH:-main}"
tap_remote_url="${HOMEBREW_TAP_REMOTE_URL:-}"
tap_token="${HOMEBREW_TAP_TOKEN:-}"
ref_type="${GITHUB_REF_TYPE:-}"
ref_name="${GITHUB_REF_NAME:-}"

notice() {
  echo "::notice title=Homebrew tap::$*"
}

if [[ ! -f "${formula_path}" ]]; then
  echo "::error title=Homebrew tap::formula file not found: ${formula_path}" >&2
  exit 1
fi

if [[ "${ref_type}" != "tag" ]]; then
  notice "skipping tap publication because this run is not a tag release"
  exit 0
fi

if [[ -z "${tap_remote_url}" ]]; then
  if [[ -z "${tap_token}" ]]; then
    notice "skipping tap publication because HOMEBREW_TAP_TOKEN is not configured"
    exit 0
  fi
  tap_remote_url="https://x-access-token:${tap_token}@github.com/${tap_repo}.git"
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

tap_dir="${tmpdir}/tap"
git clone --depth 1 --branch "${tap_branch}" "${tap_remote_url}" "${tap_dir}"

mkdir -p "${tap_dir}/Formula"
cp "${formula_path}" "${tap_dir}/Formula/browser-use-rs.rb"

git -C "${tap_dir}" config user.name "${GITHUB_ACTOR:-github-actions[bot]}"
git -C "${tap_dir}" config user.email "${GITHUB_ACTOR_EMAIL:-41898282+github-actions[bot]@users.noreply.github.com}"
git -C "${tap_dir}" add Formula/browser-use-rs.rb

if git -C "${tap_dir}" diff --cached --quiet; then
  notice "tap formula already matches ${ref_name:-the generated formula}"
  exit 0
fi

git -C "${tap_dir}" commit -m "Update browser-use-rs formula for ${ref_name:-release}"
git -C "${tap_dir}" push origin "HEAD:${tap_branch}"
notice "published Formula/browser-use-rs.rb to ${tap_repo}@${tap_branch}"
