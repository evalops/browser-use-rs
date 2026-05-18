# Release Automation

The `Release` workflow owns both version cutting and release publication.

## Cut a Stable Release

1. Open **Actions -> Release -> Run workflow**.
2. Choose `patch`, `minor`, or `major`.
3. Optionally provide an exact SemVer version, with or without the `v` prefix.
4. Run the workflow from `main`.

For workflow-dispatched cuts, the workflow:

- reads the current shared Cargo workspace version,
- computes the next SemVer version or validates the exact version,
- updates `[workspace.package].version`,
- validates that every crate inherits `version.workspace = true`,
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
python3 scripts/release-version.py --release-type minor
```

The first command verifies version consistency. The second previews the next
minor version without writing files.
