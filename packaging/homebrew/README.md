# Homebrew Formula Scaffold

Tagged releases generate `browser-use-rs.rb` from
`packaging/homebrew/browser-use-rs.rb.template` and publish it beside the
release tarball and `SHA256SUMS`.

Tagged releases call `packaging/homebrew/publish-tap.sh` after the GitHub
release is created. If `HOMEBREW_TAP_TOKEN` is not configured, the workflow
emits a notice and leaves the generated formula attached as a release artifact.

The expected tap repository is `evalops/homebrew-tap`, which Homebrew exposes as
`evalops/tap`. Configure a repository secret named `HOMEBREW_TAP_TOKEN` with
write access to that tap before expecting tagged releases to publish
`Formula/browser-use-rs.rb`. Optional repository variables
`HOMEBREW_TAP_REPOSITORY` and `HOMEBREW_TAP_BRANCH` can override the tap target.

The generated formula installs the Linux x86_64 release tarball on Linux and
the macOS release tarball on matching macOS Homebrew hosts. Until the tap exists
and the token is configured, the release asset remains the handoff artifact.
