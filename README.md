# browser-use-rs

`browser-use-rs` is an EvalOps Rust port of
[`browser-use`](https://github.com/browser-use/browser-use), designed as a
behavioral conformance implementation rather than a line-by-line translation.

The first frozen upstream target is:

```text
browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd
```

## Status

This repository is an active public Rust conformance port. Current support
includes:

- Typed Rust contracts for browser state, DOM state, actions, action results,
  LLM requests, and agent history.
- CDP-backed Chrome/Chromium launch/connect, tabs, navigation, screenshots,
  PDF capture, uploads, indexed actions, browser-profile URL access policies
  for navigation, navigation-capable actions, redirects, and newly observed
  tabs, and page state with browser-use-style short tab ids,
  including cached observed-node click/input/scroll/dropdown/upload resolution
  when available.
- DOM and accessibility-oriented snapshot serialization for browser-use's
  numbered element/action model, including open shadow DOM, same-origin iframe
  tags and contents, accessibility-tree role/name/state/value enrichment,
  backend and frontend node ids, accessible labels, image-alt control names,
  selected dropdown values, bounds, automation-friendly data/ARIA/value
  attributes, validation patterns, `data-state`,
  input mask/autocomplete/date-format hints, static history-matching
  attributes, hidden-element and `data-browser-use-exclude` subtree filtering,
  hidden file-input upload targets, and scrollable element metadata, plus common
  ARIA widget roles, search affordance signals, small icon controls,
  cursor-pointer controls, static mouse/keyboard handler attributes, pagination
  affordances, and configurable prompt-visible attributes.
- DOM indexing recognizes controls backed only by JavaScript click/pointer
  listeners when Chrome's command-line inspection API is available.
- Built-in tools: `navigate`, `search`, `click`, `input`, page/indexed
  `scroll`, text-target scroll, browser JavaScript evaluation, `wait`,
  `send_keys` for text, special keys, and shortcuts, `upload_file`,
  text/PDF/DOCX read support, text-file write/replace, PNG/JPEG image-file read
  payloads, `screenshot` with optional PNG file save, `save_as_pdf` with
  filename normalization, `extract`, `search_page`, `find_elements`, back
  navigation, 4-character tab-id actions, native/ARIA dropdown actions, and
  `done` with requested text-file display attachments.
- Agent loop: state construction, schema-guided LLM output, bounded runs,
  vision-aware browser-state capture, screenshot action next-observation image
  prompts, action-result image prompt parts, upstream-style page-stat prompt
  context with loading/skeleton hints, one-time extraction replay handling,
  step/LLM timeouts, max-failure handling,
  upstream-style initial actions, upstream-style max-action truncation,
  page-change guards, normalized repeated-action loop detection,
  loop-awareness prompt nudges, an
  upstream-style final `done` response after repeated failures, upstream
  flattened planning fields,
  configurable planning prompt nudges, thinking/flash output-schema controls,
  upstream-style flattened required output fields, structured extraction
  metadata, per-step timing metadata, upstream-style prompt-history inclusion
  and limits, clickable-element text limits, upstream-style one-time read-state
  prompt blocks, upstream-style tagged agent-history/agent-state/browser-state
  prompt sections, upstream-style available-file-path and sensitive-data
  placeholder context with `bu_2fa_code` TOTP generation, system-message
  override/extension controls, upstream-style prompt context and error
  truncation, upstream-style last-result completion helpers, upstream-compatible
  action-result success validation, judgement results, step-error,
  model-output/action/thought, duration, action, truncated action-history, and
  screenshot/URL history accessors.
- OpenAI-compatible Chat Completions, DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, Vercel AI Gateway, Anthropic Messages, Gemini GenerateContent,
  and Ollama Chat providers, including provider-specific structured-output
  modes for DeepSeek and Cerebras.
- CLI commands, stdio MCP server, local TCP/HTTP JSON-RPC daemon with optional
  bearer/header auth, supervisor pid/ready files, packaged systemd/launchd
  templates, persistent session registry, typed MCP/CLI agent settings
  including available-file-path and sensitive-data placeholder context plus
  system-message control, and conformance fixtures.

## Design Rules

- Preserve behavior and contracts before optimizing API aesthetics.
- Prefer typed contracts, explicit timeouts, and cancellable async boundaries.
- Treat browser-use Python tests and docs as conformance inputs.
- Keep small commits pushed frequently so every slice is rollbackable.
- Attribute upstream clearly and keep compatibility drift visible.

## Workspace

- `browser-use-core`: agent state, history, settings, and shared result types.
- `browser-use-cdp`: browser launch/connect/session primitives.
- `browser-use-dom`: DOM, accessibility, and selector-map types.
- `browser-use-tools`: built-in action schemas and registry contracts.
- `browser-use-llm`: provider trait and model request/response types.
- `browser-use-cli`: command-line entrypoint and daemon surface.
- `browser-use-mcp`: MCP bridge.
- `browser-use-conformance`: golden fixtures and parity test utilities.

## Roadmap

The active roadmap lives in [docs/ROADMAP.md](docs/ROADMAP.md) and the
repository issue tracker.

## CLI

The CLI includes one-shot browser commands, persistent local sessions, MCP
stdio, and a local TCP or HTTP JSON-RPC daemon.

See [docs/CLI.md](docs/CLI.md).

The MCP stdio server and contract surface are documented in
[docs/MCP.md](docs/MCP.md).

Packaged systemd and launchd supervision templates are documented in
[docs/DAEMON_SUPERVISION.md](docs/DAEMON_SUPERVISION.md).

## Install

From source:

```sh
cargo install --path crates/browser-use-cli
browser-use-rs version-target
```

Tagged GitHub releases publish a Linux x86_64 tarball containing the
`browser-use-rs` binary, license files, release support matrix, and daemon
supervision templates.

See [docs/RELEASE.md](docs/RELEASE.md) for the current supported and unsupported
browser-use surface.

## Smokes

```sh
cargo run -q -p browser-use-cli -- state \
  "data:text/html,<html><head><title>smoke</title></head><body><button>Run</button><input placeholder='Name'></body></html>"

cargo run -q -p browser-use-cli -- mcp-tools | jq -r '.[].name'

tmp=$(mktemp -d)
BROWSER_USE_RS_STATE_DIR="$tmp" cargo run -q -p browser-use-cli -- session start smoke \
  "data:text/html,<html><head><title>session smoke</title></head><body><button>Run</button></body></html>"
BROWSER_USE_RS_STATE_DIR="$tmp" cargo run -q -p browser-use-cli -- session stop smoke
rm -rf "$tmp"
```

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
