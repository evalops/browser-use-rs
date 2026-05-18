# Install

`browser-use-rs` currently publishes Linux x86_64 and macOS release binaries
plus source install paths.

## From Source

```sh
cargo install --git https://github.com/evalops/browser-use-rs --package browser-use-cli
browser-use-rs version-target
```

From a local checkout:

```sh
cargo install --path crates/browser-use-cli
browser-use-rs version-target
```

## Release Tarball

Tagged releases attach:

- `browser-use-rs-<tag>-<host-triple>.tar.gz`
- `SHA256SUMS`
- `browser-use-rs.rb`

Current release triples include `x86_64-unknown-linux-gnu` and the macOS
runner host triple, such as `aarch64-apple-darwin` or
`x86_64-apple-darwin`.

Verify and install manually:

```sh
archive=browser-use-rs-<tag>-<host-triple>.tar.gz
grep "  ${archive}$" SHA256SUMS > "${archive}.sha256"
sha256sum -c "${archive}.sha256" # or: shasum -a 256 -c "${archive}.sha256"
tar -xzf "${archive}"
./browser-use-rs version-target
```

## Homebrew

Tagged releases generate a Linux Homebrew formula artifact:

```sh
brew install ./browser-use-rs.rb
browser-use-rs version-target
```

The formula points at the Linux release tarball and pins its SHA-256 checksum.
macOS users should use the macOS tarball or source install until the Homebrew
formula learns to select macOS release assets. Maintained tap publication,
distro packages, and installer-managed secret stores are still future packaging
work.
