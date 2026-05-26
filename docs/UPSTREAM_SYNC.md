# Upstream Sync

`browser-use-rs` pins a frozen `browser-use/browser-use` commit and claims
compatibility against that target. The pin must move only after the upstream
delta has been audited and either implemented or documented as an intentional
compatibility boundary.

The `Upstream Drift` workflow runs daily and can also be dispatched manually. It
compares `INITIAL_UPSTREAM_COMMIT` with the upstream repository's default-branch
`HEAD`.

- If the SHAs match, the workflow exits without touching issues.
- If upstream moved, the workflow creates one `upstream-drift` issue or edits
  the existing open drift issue in place.
- The issue body includes the current target, latest upstream commit, compare
  URL, GitHub compare metadata, changed-file surface buckets, and a per-file
  disposition checklist.
- The workflow does not change source, docs, versions, or release tags.

When resolving a drift issue:

1. Review every file row in the generated drift issue.
2. Mark each row with exactly one disposition: implemented, not applicable, or
   deferred.
3. Split any deferred public behavior into focused parity issues.
4. Implement in-scope changes or record explicit compatibility boundaries.
5. Update `INITIAL_UPSTREAM_COMMIT` plus all docs target references in the same
   reviewed slice.
6. Run `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, and the release-helper checks.
