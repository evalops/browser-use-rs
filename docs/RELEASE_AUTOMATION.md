# Release Automation

The `Release` workflow owns both version cutting and release publication. It
runs automatically only for public-artifact `main` pushes, manually from
Actions, or on release tags.

## Automatic Update Releases

Candidate `main` pushes are changes that might affect the published binary,
packaged install assets, Cargo resolution, or release artifact docs. CI still
validates release-helper and workflow changes, but those automation-only edits
do not wake the release workflow on `main`.

Release-worthy changes are public artifact changes: workspace manifests and
lockfiles, Rust crates, packaged Homebrew/systemd/launchd assets, license/notice
files, the Rust toolchain pin, README, and public docs that ship in the package
or release support matrix. Roadmap-only, CI-only, release-workflow-only, and
release-helper-only changes do not publish unless a human manually dispatches a
release.

When one of those candidate paths changes, the workflow runs `release_type=auto`.
Auto mode compares `HEAD` with the latest stable `vX.Y.Z` tag, skips
release-bookkeeping churn, and cuts a release only when release-worthy files
changed. This keeps manual reruns and historical workflow changes from creating
accidental empty releases.

Auto mode chooses:

- `major` when an unreleased commit contains a breaking-change marker such as
  `BREAKING CHANGE` or a Conventional Commit `!`.
- `minor` when unreleased work has a substantial public-behavior signal:
  `Release-Impact: minor`, a Conventional Commit `feat:` subject, source/test
  changes paired with README/conformance/CLI/MCP/install/release docs for the
  new capability, or broad cross-crate public-surface work.
- `patch` for smaller release-worthy changes: fixes, dependency or toolchain
  refreshes, packaged install asset changes, README/support-matrix updates, and
  public docs that should ship with the next artifact. Small compatibility
  aliases and narrowly scoped fixes should use `Release-Impact: patch` when
  their commit message could otherwise look like new feature work.

For ambiguous commits, add a trailer to the commit body:

```text
Release-Impact: minor
Release-Impact: patch
Release-Impact: none
```

`Release-Impact` trailers override the heuristic. Use `minor` for substantial
new user-visible behavior, `patch` for small but releasable changes, and `none`
for maintenance that should never publish by itself. If multiple unreleased
commits request a release, the highest requested impact wins. This makes the
workflow release by the meaning of the work, not by how many rollback commits
have landed since the last tag.

If a manual auto run finds nothing release-worthy after the latest stable tag,
the run exits successfully without committing, tagging, building, or publishing.
When it does publish, the GitHub release body starts with the commits since the
previous stable tag and then appends the release support matrix.

## Cut a Stable Release Manually

1. Open **Actions -> Release -> Run workflow**.
2. Choose `auto`, `patch`, `minor`, or `major`.
3. Optionally provide an exact SemVer version, with or without the `v` prefix.
4. Run the workflow from `main`.

For workflow-dispatched cuts, the workflow:

- reads the current shared Cargo workspace version,
- computes the next SemVer version, infers it from unreleased changes, or
  validates the exact version,
- updates `[workspace.package].version`,
- validates that every crate inherits `version.workspace = true`,
- refreshes `Cargo.lock` before tagging so the tag contains the workspace package
  versions used by the build,
- commits `Cut browser-use-rs vX.Y.Z` to `main`,
- creates the matching annotated `vX.Y.Z` tag,
- builds Linux and macOS release tarballs from that tag,
- publishes the GitHub release with checksums and the generated Homebrew formula,
- publishes the EvalOps Homebrew tap formula when `HOMEBREW_TAP_TOKEN` is configured.

## Version Rules

All workspace crates inherit the root `[workspace.package].version`; release
automation must not edit crate versions independently.

Automatic `patch`, `minor`, and `major` cuts produce stable `X.Y.Z` versions.
Exact versions may include a prerelease suffix such as `0.2.0-alpha.1`, but the
requested version must be greater than the current workspace version.

Before publishing assets, the release workflow checks that the tag version
matches the Cargo workspace version. This keeps binary `CARGO_PKG_VERSION`, MCP
metadata, tarball names, GitHub release tags, and Homebrew formula URLs aligned.

## Local Checks

Use the release helper before editing release workflows or Cargo manifests:

```sh
python3 scripts/release-version.py --check
python3 scripts/release-version.py --self-test
python3 scripts/release-version.py --release-type auto --allow-no-release
python3 scripts/release-version.py --release-type minor
```

The first command verifies version consistency. The second exercises the
release-impact classifier. The third previews whether the current unreleased
changes warrant a release and which bump they would receive. The fourth previews
the next minor version without writing files.
