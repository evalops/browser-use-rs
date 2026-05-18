# browser-use-rs

`browser-use-rs` is an EvalOps Rust port of
[`browser-use`](https://github.com/browser-use/browser-use), designed as a
behavioral conformance implementation rather than a line-by-line translation.

The current frozen upstream target is:

```text
browser-use/browser-use@f09a86671591312bbc272403a7409d56f4cec668
```

## Status

This repository is an active public Rust conformance port. Current support
includes:

- Typed Rust contracts for browser state, DOM state, actions, action results,
  LLM requests, and agent history.
- CDP-backed Chrome/Chromium launch/connect, tabs, navigation, screenshots,
  PDF capture, uploads, indexed actions, browser-profile URL access policies
  for navigation, blocked-navigation preflight diagnostics,
  navigation-capable actions, redirects, and newly observed tabs, and page
  state with browser-use-style short tab ids,
  plus typed Browser Use Cloud creation parameters and response-to-CDP endpoint
  conversion that preserve upstream's omitted/null/proxy-country distinction,
  including cached observed-node click/input/scroll/dropdown/upload resolution
  and target-aware stale-node fallback for cached iframe actions when
  available. CDP sessions expose a bounded public lifecycle event history for
  browser connect/close, target create/switch/close, navigation
  start/complete/failure/timeout, target crash, URL-policy reset/popup
  diagnostics, reconnect, JavaScript dialog, download, and storage-state
  diagnostics plus a `BrowserLifecycleEventSubscription` returned by
  `subscribe_lifecycle_events`, with `recv`/`try_recv` lag and closed-stream
  handling, and a `BrowserLifecycleAdapterEventSubscription` that maps the
  diagnostics into upstream-style adapter categories without adding the full
  event stream to normal agent replies.
  CDP websocket closure is recorded as a browser-stopped lifecycle diagnostic,
  and unexpected websocket drops trigger bounded actor-level reconnect attempts
  with reconnecting/reconnected/failure lifecycle diagnostics. Registered CDP
  target sessions are invalidated after reconnect so stale session-scoped
  commands fail locally with a clear reattach error, and the current target is
  reattached automatically on the next session access when Chrome still exposes
  it.
  `navigation_timeout_ms` bounds direct `Page.navigate` calls and records
  network-timeout lifecycle diagnostics when they hang.
  `network_request_timeout_ms` records lifecycle diagnostics for HTTP(S)
  requests that remain active beyond the watchdog budget.
  Launch profiles can set `downloads_path` to enable Chrome download behavior
  and browser-level download lifecycle events, and `storage_state_path` to
  load/save browser cookie and attached frame-tree origin local/session storage
  state with lifecycle notifications. Profile-wide storage discovery outside
  the attached frame tree is outside the safe CDP boundary documented in
  [docs/CONFORMANCE.md](docs/CONFORMANCE.md).
- DOM and accessibility-oriented snapshot serialization for browser-use's
  numbered element/action model, including open shadow DOM, same-origin iframe
  tags and contents, Chrome OOPIF cross-origin iframe target contents and
  cached-node actions, accessibility-tree role/name/description/state/value
  enrichment, compact `ax_name`/`ax_description` metadata, AX hidden/disabled
  suppression,
  backend and frontend node ids, accessible labels, image-alt control names,
  selected dropdown values, compound control metadata, compact select option
  summaries, bounds, automation-friendly data/ARIA/value attributes,
  native boolean/read-only state, validation patterns, `data-state`,
  input mask/autocomplete/date-format datepicker hints, live-region and
  hierarchy metadata, static history-matching attributes, plus a tree-shaped
  eval/judge DOM representation with upstream-style backend-node markers,
  hidden-element and `data-browser-use-exclude` subtree filtering, non-content
  `head`/`script`/`style` tag pruning, occluded-element filtering, hidden
  file-input upload targets, plain scroll-container indexing, and scrollable
  element metadata with prompt-visible pages-above/below context,
  plus href-less anchor tags, common ARIA widget roles, search affordance
  signals, tabindex-backed controls including `tabindex="-1"`, ARIA
  required/autocomplete/keyshortcut interactivity signals with prompt-visible
  `keyshortcuts`, quiet AX focusable/editable/settable metadata,
  AX-shaped numeric value aliases,
  human-readable value text, contenteditable editor variants, media control
  compounds, small icon controls, cursor-pointer controls,
  decorative SVG child pruning, contained duplicate descendant pruning for
  action containers, static mouse/keyboard handler attributes, pagination
  affordances, duplicate long-attribute pruning, and configurable
  prompt-visible attributes.
- DOM indexing recognizes controls backed only by JavaScript click/pointer
  listeners when Chrome's command-line inspection API is available.
- Built-in tools: `navigate`, `search`, `click`, `input`, page/indexed
  `scroll`, text-target scroll, browser JavaScript evaluation, `wait`,
  `send_keys` for text, special keys, and shortcuts, `upload_file` with
  upstream-style agent availability checks,
  text/PDF/DOCX read support with page-aware PDF envelopes, text-file
  write/replace with CSV normalization and relative filename sanitization,
  PDF/DOCX write/append artifacts with paginated PDF text layout, and
  upstream-style append semantics, PNG/JPEG image-file read payloads,
  upstream-aligned binary/image extension rejection,
  `screenshot` with optional PNG file save, `save_as_pdf` with filename
  normalization, `extract`, `search_page`, `find_elements` including
  target-aware stale-node action fallback for Chrome OOPIF iframe targets, back
  navigation, 4-character tab-id actions, native/ARIA dropdown actions, and
  `done` with requested text-file display attachments.
- Managed `FileSystem` state with a `browseruse_agent_data` sandbox directory,
  default `todo.md`, file listing/display, extract-content numbering,
  serialization/restoration, nuke, and disk sync for text, CSV, PDF, and DOCX
  artifacts. Executor-owned relative `write_file`/append/`read_file`/
  `replace_file` and `done.files_to_display` flows route through the sandbox
  while absolute external paths continue to bypass it. Agent prompts include
  upstream-style `<file_system>` and `<todo_contents>` context, and large
  extract results can spill into managed `extracted_content_N.md` files.
  Restored agents can continue prompt and tool execution from serialized
  `FileSystemState`, including preserved todo/report context and incrementing
  extracted-content numbering.
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
  configurable planning prompt nudges, upstream-style excluded-action schema
  controls and pre-execution enforcement, opt-in recent browser events,
  upstream-style `true`/`false`/`auto` vision modes with auto-only screenshot
  action gating, upstream-style vision detail levels, upstream-style `done`
  file-display controls, thinking/flash output-schema controls,
  upstream-style flattened required output fields, structured extraction
  metadata, per-step timing metadata, upstream-style prompt-history inclusion
  and limits, clickable-element text limits, upstream-style one-time read-state
  prompt blocks, upstream-style tagged agent-history/agent-state/browser-state
  prompt sections, upstream-style available-file-path and sensitive-data
  placeholder context with `bu_2fa_code` TOTP generation, system-message
  override/extension controls, upstream-style prompt context and error
  truncation, upstream-style last-result completion helpers, upstream-compatible
  action-result success validation, judgement results, runtime `generate_gif`
  GIF artifact output from recorded screenshots, contract-preserved
  `calculate_cost` and `include_tool_call_examples` settings, step-error,
  model-output/action/thought, duration, action, model-action and truncated
  action-history interacted-element metadata, and screenshot/URL history
  accessors. Agents can export a serializable
  `AgentCheckpoint` and resume it with a new model/session while preserving
  task settings, history, initial-action execution state, and managed
  filesystem state. Conformance fixtures include a longer multi-step replay for
  planning nudges, recovery after a failed browser action, prompt-history
  limits, stagnant-page loop-awareness, interacted-element rematching,
  action-level replay remapping diagnostics, rematched replay-plan construction,
  generic and browser-guarded replay execution diagnostics, current-state
  `AgentHistoryReplayRun` orchestration with DOM recapture between replay
  actions, replay-run and replay-recapture JSON conformance fixtures,
  replay-run JSON Schema snapshot, file artifacts, and final `done`.
- OpenAI-compatible Chat Completions, DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, Vercel AI Gateway, Anthropic Messages, Gemini GenerateContent,
  and Ollama Chat providers, including provider-specific structured-output
  modes for DeepSeek forced tool calls, Cerebras prompt-only guidance, and
  OpenRouter app attribution headers.
- CLI commands including one-shot and persistent-session history replay, stdio
  MCP server, local TCP/HTTP JSON-RPC daemon, MCP/daemon history replay,
  optional bearer/header auth, supervisor pid/ready files, packaged
  systemd/launchd templates, persistent session registry for explicit and
  implicit `session_id` MCP calls with liveness status and stale-record
  cleanup, MCP input/output schemas, typed MCP/CLI agent settings including
  conversation transcript saving, judge trace validation, available-file-path
  and sensitive-data placeholder context, upstream-style message compaction
  controls plus system-message control, and conformance fixtures.

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

The CLI includes one-shot browser commands, history replay, persistent local
sessions, MCP stdio, and a local TCP or HTTP JSON-RPC daemon.

See [docs/CLI.md](docs/CLI.md).

The MCP stdio server and contract surface are documented in
[docs/MCP.md](docs/MCP.md).

Packaged systemd and launchd supervision templates are documented in
[docs/DAEMON_SUPERVISION.md](docs/DAEMON_SUPERVISION.md).

## Install

From source:

```sh
cargo install --git https://github.com/evalops/browser-use-rs --package browser-use-cli
browser-use-rs version-target
```

From a local checkout, use `cargo install --path crates/browser-use-cli`.

Tagged GitHub releases publish Linux x86_64 and macOS host-triple tarballs,
`SHA256SUMS`, and a platform-aware Homebrew formula artifact. When the EvalOps
tap is configured, tagged releases also publish `Formula/browser-use-rs.rb` to
`evalops/homebrew-tap` for `brew tap evalops/tap && brew install browser-use-rs`.
The tarballs contain the `browser-use-rs` binary, license files, install guide,
release support matrix, and daemon supervision templates.

See [docs/INSTALL.md](docs/INSTALL.md) for install commands and
[docs/RELEASE.md](docs/RELEASE.md) for the current supported and unsupported
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
