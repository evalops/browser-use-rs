# browser-use-rs

`browser-use-rs` is the EvalOps Rust port of
[`browser-use`](https://github.com/browser-use/browser-use). It aims for
behavioral compatibility at the public contract level rather than a line-by-line
translation of the Python internals.

Current frozen upstream target:

```text
browser-use/browser-use@157779338afdcc03023010ec3c24ad63d820453c
```

The detailed support matrix lives in [docs/RELEASE.md](docs/RELEASE.md). The
upstream sync process is documented in [docs/UPSTREAM_SYNC.md](docs/UPSTREAM_SYNC.md).

## What Works

- CDP-backed local Chrome/Chromium launch and attach, with browser profile
  controls for launch arguments, viewport, downloads, permissions, storage
  state, HAR/video/trace artifacts, URL access policy, and lifecycle
  diagnostics.
- Browser state capture with compact DOM and accessibility metadata, numbered
  selector maps, iframe and shadow DOM support, occlusion filtering, scroll
  context, and target-aware cached-node fallback.
- Built-in browser actions for navigation, search, click, input, scroll, file
  upload, keyboard input, tab management, JavaScript evaluation, screenshots,
  PDF capture, extraction, file operations, dropdowns, and `done`.
- Agent execution with schema-guided LLM output, bounded steps and timeouts,
  initial actions, vision modes, message history, replay/rematching,
  checkpoints, managed filesystem state, callbacks, pause/resume, follow-up
  tasks, judgement routing, token usage summaries, and GIF generation.
- LLM adapters for OpenAI-compatible chat completions, DeepSeek, Groq,
  Cerebras, Mistral, OpenRouter, Vercel AI Gateway, Anthropic, Gemini, and
  Ollama, including provider-specific structured-output fallbacks.
- CLI, MCP stdio, persistent local sessions, local TCP/HTTP JSON-RPC daemon,
  typed MCP schemas, packaged systemd/launchd templates, and release tarballs
  for Linux and macOS.

## Install

Install from the public repository:

```sh
cargo install --git https://github.com/evalops/browser-use-rs --package browser-use-cli
browser-use-rs version-target
```

Install from a local checkout:

```sh
cargo install --path crates/browser-use-cli
```

Tagged GitHub releases publish Linux x86_64 and macOS host-triple tarballs,
`SHA256SUMS`, and a generated Homebrew formula artifact. See
[docs/INSTALL.md](docs/INSTALL.md) for release tarball and Homebrew details.

## Quick Start

Capture browser state:

```sh
cargo run -q -p browser-use-cli -- state \
  "data:text/html,<html><head><title>smoke</title></head><body><button>Run</button><input placeholder='Name'></body></html>"
```

List MCP tools:

```sh
cargo run -q -p browser-use-cli -- mcp-tools | jq -r '.[].name'
```

Start and stop a persistent local session:

```sh
tmp=$(mktemp -d)
BROWSER_USE_RS_STATE_DIR="$tmp" cargo run -q -p browser-use-cli -- session start smoke \
  "data:text/html,<html><head><title>session smoke</title></head><body><button>Run</button></body></html>"
BROWSER_USE_RS_STATE_DIR="$tmp" cargo run -q -p browser-use-cli -- session stop smoke
rm -rf "$tmp"
```

The CLI surface is documented in [docs/CLI.md](docs/CLI.md), MCP in
[docs/MCP.md](docs/MCP.md), and daemon supervision in
[docs/DAEMON_SUPERVISION.md](docs/DAEMON_SUPERVISION.md).
The implementation map lives in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
and the file-level maintainer guide in
[docs/CODEBASE_GUIDE.md](docs/CODEBASE_GUIDE.md).

## Workspace

- `browser-use-core`: agent state, history, settings, and shared result types.
- `browser-use-cdp`: browser launch, CDP transport, sessions, targets, and tabs.
- `browser-use-dom`: DOM, accessibility, and selector-map rendering.
- `browser-use-tools`: built-in action schemas and registry contracts.
- `browser-use-llm`: provider trait and model request/response adapters.
- `browser-use-cli`: command-line entrypoint and local daemon surface.
- `browser-use-mcp`: MCP bridge.
- `browser-use-conformance`: golden fixtures and parity test utilities.

## Keeping Current

The project pins a frozen upstream commit so compatibility claims stay precise.
The `Upstream Drift` workflow checks the upstream repository daily. If
`browser-use/browser-use` moves, it opens or updates a single `upstream-drift`
issue with the compare URL and audit checklist. The pin moves only after the
delta is audited, implemented, or documented as an intentional compatibility
boundary.

Release automation is also meaning-aware: substantial public behavior can cut a
minor version, smaller releasable changes cut a patch version, and automation
only changes do not publish. See
[docs/RELEASE_AUTOMATION.md](docs/RELEASE_AUTOMATION.md).

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/release-version.py --check
python3 scripts/release-version.py --self-test
python3 scripts/upstream-drift.py --self-test
```

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
