# Release Automation

The `Release` workflow owns both version cutting and release publication. It
runs automatically only for meaningful `main` pushes, manually from Actions, or
on release tags.

## Automatic Update Releases

Meaningful `main` pushes are changes that can affect the published binary,
packaged install assets, Cargo resolution, or release artifact contents:
workspace manifests and lockfiles, Rust crates, packaged Homebrew/systemd/launchd
assets, license/notice files, and the Rust toolchain pin. Roadmap, docs, CI,
release workflow, and release-helper maintenance continue through CI but do not
wake the release publisher.

When one of those meaningful paths changes, the workflow runs `release_type=auto`.
Auto mode compares `HEAD` with the latest stable `vX.Y.Z` tag, skips
release-bookkeeping churn, and cuts a release only when release-worthy files
changed. The script keeps this second guard so manual reruns and historical
workflow changes still avoid accidental empty releases.

Auto mode chooses:

- `major` when an unreleased commit contains a breaking-change marker such as
  `BREAKING CHANGE` or a Conventional Commit `!`.
- `minor` when Rust crate behavior changed and the unreleased commits look like
  feature or public-surface additions.
- `patch` for release-worthy fixes and packaged install asset changes.

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
python3 scripts/release-version.py --release-type auto --allow-no-release
python3 scripts/release-version.py --release-type minor
```

The first command verifies version consistency. The second previews whether the
current unreleased changes warrant a release and which bump they would receive.
The third previews the next minor version without writing files.
