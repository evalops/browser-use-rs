# Homebrew Formula Scaffold

Tagged releases generate `browser-use-rs.rb` from
`packaging/homebrew/browser-use-rs.rb.template` and publish it beside the
release tarball and `SHA256SUMS`.

Ownership stays in this repository until EvalOps wires a dedicated tap. A tap
publisher should copy the generated formula into `evalops/homebrew-tap` (or a
successor tap), review the version, URL, and SHA-256, then tag or publish from
that tap.

The current generated formula installs the Linux x86_64 release tarball. macOS
Homebrew support should be added only after the release workflow publishes a
macOS binary artifact with its own checksum.
