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
  URL, and the audit checklist.
- The workflow does not change source, docs, versions, or release tags.

When resolving a drift issue:

1. Review upstream changes in `browser_use/browser`, `browser_use/dom`,
   `browser_use/agent`, `browser_use/llm`, `browser_use/mcp`, and CLI/package
   entrypoints.
2. Split new or changed public behavior into focused parity issues.
3. Implement those issues or record explicit compatibility boundaries.
4. Update `INITIAL_UPSTREAM_COMMIT` plus all docs target references in the same
   reviewed slice.
5. Run `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`, and the release-helper checks.
