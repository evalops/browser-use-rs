# Architecture

`browser-use-rs` is a behavioral Rust port of
`browser-use/browser-use`, pinned to upstream commit
`157779338afdcc03023010ec3c24ad63d820453c`.

The port is not a class-by-class translation. The public model stays close to
browser-use: an agent observes browser state, asks an LLM for structured
actions, executes those actions in a browser, records history, and stops when a
`done` action or a guard condition says it should stop. The internals are split
into typed Rust crates and modules so the compatibility surface is explicit.

## Design Rules

- Public behavior is a conformance contract. Schema shape, action names,
  browser-state semantics, prompt sections, and lifecycle diagnostics should
  change only with tests and a documented compatibility reason.
- `browser-use-core` owns agent behavior, not browser transport details.
- `browser-use-cdp` owns Chrome DevTools Protocol behavior, not prompt or LLM
  policy.
- DOM and accessibility payloads are compact model inputs, not raw CDP dumps.
- Provider quirks belong in `browser-use-llm`; action semantics belong in
  `browser-use-tools` and `browser-use-core`.
- Background tasks must be bounded, abortable by ownership drop, and observable
  through state or lifecycle diagnostics.
- Public root re-exports preserve the crate API while internal modules keep the
  implementation navigable.

## Crate Map

| Crate | Responsibility |
| --- | --- |
| `browser-use-core` | Agent loop, prompts, history, settings, managed files, action execution, replay, usage accounting, checkpoints, callbacks. |
| `browser-use-cdp` | Chrome launch/attach, Browser Use Cloud sessions, CDP transport, browser profile mapping, DOM capture, action dispatch, downloads, storage state, HAR/video/trace artifacts, lifecycle and security watchdogs. |
| `browser-use-dom` | Compact DOM state types, selector maps, action-history element identity, text rendering, DOM rematch levels. |
| `browser-use-tools` | Built-in browser action schemas and the action registry contract. |
| `browser-use-llm` | Provider-neutral chat trait plus OpenAI-compatible, Anthropic, Gemini, and Ollama adapters. |
| `browser-use-cli` | Human CLI, local session store, daemon entrypoint, JSON surfaces. |
| `browser-use-mcp` | MCP stdio bridge backed by the CLI/session/core contracts. |
| `browser-use-conformance` | Golden fixture helpers and parity utilities. |

## Runtime Flow

The main agent path is:

```text
Agent::run
  -> execute configured initial actions once
  -> capture BrowserStateSummary from BrowserSession
  -> build a ChatRequest in browser-use-core::prompt
  -> invoke ChatModel, optionally switching to fallback LLM
  -> parse AgentOutput and restore shortened URLs
  -> execute BrowserAction values through BrowserActionExecutor
  -> update AgentHistory and usage summary
  -> maybe compact history, judge done result, save transcript, save GIF
```

The CDP browser-state path is:

```text
CdpBrowserSession::state
  -> enforce URL policy and wait for page-load settle
  -> read page location and page metrics
  -> run DOM indexing JavaScript
  -> join DOMSnapshot backend ids to Accessibility.getFullAXTree
  -> parse compact SerializedDomState
  -> merge same-origin and OOPIF iframe states
  -> cache target-aware elements for stable actions
  -> return BrowserStateSummary with lifecycle/security diagnostics
```

The CDP action path is:

```text
BrowserAction
  -> BrowserActionExecutor in core
  -> BrowserSession trait method
  -> CdpBrowserSession method
  -> cached-node callFunctionOn when possible
  -> index-based fallback when the cached node is stale or detached
  -> URL-policy and page-change guards
```

## `browser-use-core`

`browser-use-core` is the agent contract crate. It re-exports the public types
that downstream callers use, while private modules hold the implementation.

| Module | Owns |
| --- | --- |
| `agent.rs` | `Agent`, `AgentTask`, checkpoints, run/step orchestration, pause/resume/stop, callbacks, fallback LLM switching, transcript and GIF output, final-response and judge handling. |
| `prompt.rs` | Step requests, final response requests, judge and compaction requests, action-output schemas, sensitive-data substitution, TOTP placeholders, previous-result rendering, prompt screenshot resizing, loop and budget warnings. |
| `history.rs` | `AgentHistory`, `AgentHistoryItem`, `AgentOutput`, action results, replay/rematch planning, compacted memory, usage summary shape, terminal-result helpers. |
| `settings.rs` | `AgentSettings`, vision modes, action and wait timeout coercion, message compaction settings, generated GIF settings, sensitive data values. |
| `file_system.rs` | Managed sandbox paths, file state serialization, text/PDF/DOCX/image file actions, result display helpers. |
| `executor.rs` | `ActionExecutor`, `BrowserActionExecutor`, browser action side effects, page extraction preparation, screenshot/PDF output helpers, replay execution helpers. |
| `urls.rs` | Task URL extraction, search URL building, prompt URL shortening, model-output URL restoration. |
| `usage.rs` | Token and cost aggregation from provider usage metadata. |
| `lib.rs` | Public re-exports and crate-level compatibility shims for tests and sibling modules. |

### Agent Loop Ownership

`agent.rs` is the only module that should decide agent control flow:

- when to call the model;
- when a step times out;
- when the fallback LLM is eligible;
- when a loop or max-failure guard stops the run;
- when callbacks run;
- when final-response-after-failure or final-step requests are used;
- when history compaction or judge validation runs.

Prompt wording and schemas stay in `prompt.rs`. Browser action side effects
stay in `executor.rs` through `BrowserActionExecutor` and the `BrowserSession`
trait.

### Prompt Ownership

`prompt.rs` is the only core module that should know the model-facing prompt
layout. It deliberately owns both text rendering and JSON schema generation so
the prompt and parser stay compatible. If a new action needs an LLM schema
change, add the schema branch, prompt rendering, and exclusion behavior here
with tests.

### Managed Files

The managed filesystem is part of agent state. It is serialized through
`AgentCheckpoint`, restored through `Agent::from_checkpoint`, and passed into
prompt construction so the model can see available files. Browser upload and
read/write actions should route through `ManagedFileSystem` instead of touching
arbitrary relative paths.

## `browser-use-cdp`

`browser-use-cdp` is the largest crate because it owns both browser process
management and live CDP session behavior.

| Module | Owns |
| --- | --- |
| `lib.rs` | Public browser primitives, CDP session state, action methods, root re-exports, `BrowserSession` trait, and compatibility tests. |
| `types.rs` | Shared public DTOs and serde helpers: errors, screenshots, PDFs, found elements, viewport/proxy settings, cloud proxy country codes. |
| `cloud.rs` | Browser Use Cloud request/response/client types, API-key discovery, auth-config lookup, cloud HTTP error rendering. |
| `profile.rs` | Browser profile serde defaults and aliases, Chrome launch plans, executable discovery, local process launch, `DevToolsActivePort` parsing. |
| `policy.rs` | Browser profile URL-access policy, allow/prohibit pattern matching, IP-address blocking, and navigation block reasons. |
| `input.rs` | Keyboard alias normalization and CDP `Input.dispatchKeyEvent` parameter construction. |
| `runtime.rs` | `Runtime.evaluate` parameter construction, value extraction, and exception/result rendering. |
| `lifecycle.rs` | Lifecycle event DTOs, upstream adapter event mapping, lifecycle subscriptions, lag/closed stream errors. |
| `transport.rs` | Websocket connection, CDP command actor, response routing, event broadcast, reconnect attempts, stale session generation checks, websocket header validation. |
| `dom.rs` | Injected DOM/action JavaScript, element highlight scripts, DOMSnapshot and accessibility joins, iframe target merging, compact DOM parsing, pagination detection, cached-index target mapping. |
| `recording.rs` | HAR capture, trace artifacts, screencast video/GIF writing, artifact path generation, recorder diagnostics. |
| `storage.rs` | Cookie/origin storage save/load, frame-origin discovery, DOMStorage conversion, storage-state counts and file writes. |
| `watchdog.rs` | Lifecycle watchdog, security watchdog, URL-policy actions, bounded event buffers, websocket lifecycle event mapping, network timeouts, download event mapping, auto-PDF download handling. |

### CDP Session Shape

`CdpBrowserSession` holds:

- `Arc<CdpConnection>` for command/event transport;
- the current attached page target and session id;
- cached DOM state and target-aware cached elements;
- security and lifecycle event buffers;
- URL access policy and pending policy errors;
- profile-derived iframe, viewport, page-load, highlight, and download config;
- optional HAR, video, and trace recorders;
- owned temporary directories for user data and downloads.

The session implements the public `BrowserSession` trait used by core. That
trait is the boundary between agent semantics and browser mechanics.

### Transport Boundary

`transport.rs` has no DOM, prompt, profile, or action policy. It only knows how
to:

- connect to the CDP websocket with optional profile headers;
- send commands with monotonically increasing ids;
- route command responses back to callers;
- broadcast CDP events;
- reconnect boundedly after unexpected websocket drops;
- mark old target sessions as stale after reconnect.

Any code that needs a browser operation should call `CdpConnection::command`
through a higher-level session method rather than adding protocol policy to the
transport actor.

### DOM Boundary

`dom.rs` owns the model-visible browser-state contract. It indexes elements
with injected JavaScript, joins accessibility metadata, parses page stats and
bounds, merges iframe target states, and detects pagination affordances.

This module intentionally emits compact state:

- numbered selector maps;
- useful names, roles, text, values, attributes, and bounds;
- compact accessibility names/descriptions and state/value properties;
- target ids for iframe and stale-node fallback;
- eval tree data needed by evaluator-style prompts.

It should not expose raw `DOMSnapshot` or `Accessibility` trees to normal agent
prompts. Add raw payloads only behind an explicit diagnostic or conformance
surface.

### Watchdog Boundary

`watchdog.rs` owns asynchronous browser safety and observability tasks:

- lifecycle event collection and bounded publication;
- websocket closed/reconnecting/reconnected/failure events;
- target crash and JavaScript dialog handling;
- network request timeout diagnostics;
- download start/progress/completion mapping;
- auto-PDF response-body capture;
- URL-policy reset/close actions for blocked current tabs and popups.

Watchdogs are owned by `CdpBrowserSession`; dropping the session aborts their
tasks. Watchdog diagnostics are available through lifecycle subscriptions and
selected state fields, but they are not added to normal agent answers.

`policy.rs` owns the pure URL decision logic used by both the session boundary
and the security watchdog. Session code may record and surface policy failures,
but allowlist/prohibit matching and IP canonicalization should stay in
`policy.rs`.

`runtime.rs` and `input.rs` own protocol value shaping for JavaScript
evaluation and keyboard events. Session methods choose when to evaluate or
dispatch; these modules decide how CDP payloads and responses are represented.

`types.rs` owns DTOs that are shared across CDP submodules or exported publicly.
Keep serde compatibility helpers next to the DTOs they shape, then re-export the
public API from `lib.rs`.

### Profile, Recording, And Storage Boundaries

`profile.rs` translates user-facing browser profile options into launch plans
and cloud/local endpoints. It should not know about live target state after a
session starts.

`recording.rs` observes CDP events and writes optional artifacts. Recording
failures become lifecycle diagnostics; they should not change browser action
semantics unless a required CDP command fails.

`storage.rs` owns the browser storage-state contract. Session methods may ask it
to read, write, or apply state, but cookie/origin normalization should stay in
that module.

## `browser-use-dom`

`browser-use-dom` is intentionally data-focused. It defines:

- `SerializedDomState`;
- `DomElementRef`;
- `DomEvalNode`;
- `DomInteractedElement` and rematch metadata;
- `BrowserStateSummary`;
- page stats, page info, tabs, and pagination DTOs;
- compact text rendering for prompts and history.

Core and CDP both depend on these types. DOM state should stay serializable and
small enough to be used in prompt fixtures.

## `browser-use-tools`

`browser-use-tools` defines the browser action enum and action parameter DTOs:
navigation, search, click, input, scroll, keyboard, tabs, upload, screenshot,
PDF, extraction, file operations, dropdowns, JavaScript evaluation, and `done`.

Action names and schema shape are part of the upstream compatibility contract.
Runtime semantics are split: schema lives here, prompt/exclusion handling lives
in `browser-use-core::prompt`, and browser side effects live behind the
`BrowserSession` implementation.

## `browser-use-llm`

`browser-use-llm` exposes `ChatModel` and provider-neutral message/request/
completion types. Provider modules translate that contract into concrete APIs.

Provider-specific structured-output quirks belong here. The agent should not
know whether a model needs forced tool use, schema sanitization, prompt-only
fallback, wrapped-JSON recovery, or a provider-specific endpoint.

## CLI, MCP, And Conformance

`browser-use-cli` is both a human entrypoint and the owner of local persistent
session process state. `browser-use-mcp` exposes the same operations as MCP
tools. These front doors should avoid duplicating core agent or CDP session
logic; they adapt IO, persistence, and error presentation.

`browser-use-conformance` and the tests embedded in each crate hold parity
fixtures. When in doubt, add a fixture or regression test before changing a
contract.

## Public API Strategy

The root modules re-export public types to avoid breaking downstream users as
the implementation gets more idiomatic internally. New modules may expose
`pub(crate)` helpers for cross-module tests or crate-internal collaboration,
but new public exports should be deliberate.

Compatibility-sensitive public exports include:

- core agent types and settings;
- action schemas;
- browser profile and lifecycle DTOs;
- `BrowserSession`;
- provider request/response DTOs;
- CLI and MCP JSON shapes.

## Where To Put New Work

- New prompt text or schema: `browser-use-core/src/prompt.rs`.
- New agent control-flow guard: `browser-use-core/src/agent.rs`.
- New browser action schema: `browser-use-tools/src/lib.rs`.
- New browser action execution: `browser-use-core/src/executor.rs` and the
  `BrowserSession` trait implementation in `browser-use-cdp/src/lib.rs`.
- New DOM state field: `browser-use-dom/src/lib.rs` plus parser/rendering in
  `browser-use-cdp/src/dom.rs` and prompt use in `browser-use-core/src/prompt.rs`.
- New CDP command transport behavior: `browser-use-cdp/src/transport.rs`.
- New browser safety/lifecycle behavior: `browser-use-cdp/src/watchdog.rs`.
- New URL policy behavior: `browser-use-cdp/src/policy.rs`.
- New Runtime.evaluate or keyboard payload behavior:
  `browser-use-cdp/src/runtime.rs` or `browser-use-cdp/src/input.rs`.
- New profile/cloud launch behavior: `browser-use-cdp/src/profile.rs` or
  `browser-use-cdp/src/cloud.rs`.
- New artifact or storage behavior: `browser-use-cdp/src/recording.rs` or
  `browser-use-cdp/src/storage.rs`.
- New provider behavior: `browser-use-llm/src/lib.rs`.
- New CLI/MCP surface: `browser-use-cli/src/main.rs` and
  `browser-use-mcp/src/lib.rs`.
- New CDP public DTO or serde-helper behavior: `browser-use-cdp/src/types.rs`.

## Verification Gates

Use the narrowest useful command while developing, then run the full gate
before shipping:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p browser-use-cdp -- --ignored
python3 scripts/release-version.py --check
python3 scripts/release-version.py --self-test
python3 scripts/upstream-drift.py --self-test
```

Ignored CDP tests require Chrome or Chromium. They are the live conformance
check for DOM indexing, target fallback, URL policy watchdogs, and browser
actions.

## Non-Goals

- A literal Python class hierarchy.
- Raw CDP payloads in normal prompts or agent replies.
- Provider-specific shortcuts in core agent logic.
- Hidden unbounded watchers.
- Silent upstream drift.
