# Install

`browser-use-rs` currently publishes a Linux x86_64 release binary plus source
install paths.

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

- `browser-use-rs-<tag>-x86_64-unknown-linux-gnu.tar.gz`
- `SHA256SUMS`
- `browser-use-rs.rb`

Verify and install manually:

```sh
sha256sum -c SHA256SUMS
tar -xzf browser-use-rs-<tag>-x86_64-unknown-linux-gnu.tar.gz
./browser-use-rs version-target
```

## Homebrew

Tagged releases generate a Linux Homebrew formula artifact:

```sh
brew install ./browser-use-rs.rb
browser-use-rs version-target
```

The formula points at the release tarball and pins its SHA-256 checksum.
Maintained tap publication, macOS prebuilt artifacts, distro packages, and
installer-managed secret stores are still future packaging work.
