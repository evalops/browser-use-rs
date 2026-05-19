# Codebase Guide

This guide is the maintainer map for `browser-use-rs`. It complements
`ARCHITECTURE.md` by listing the concrete files, contracts, and review risks
for each part of the workspace.

## Top-Level Layout

```text
crates/
  browser-use-core/         Agent loop and shared agent contracts
  browser-use-cdp/          Chrome/CDP browser implementation
  browser-use-dom/          Compact DOM and browser-state DTOs
  browser-use-tools/        Built-in action schemas
  browser-use-llm/          Provider-neutral chat model and adapters
  browser-use-cli/          CLI, daemon, local session store
  browser-use-mcp/          MCP stdio bridge
  browser-use-conformance/  Fixture helpers
docs/                       Architecture, conformance, install, release, CLI, MCP
scripts/                    Release and upstream-drift automation
fixtures/                   Golden compatibility fixtures
```

## Core Crate

Path: `crates/browser-use-core/src`.

### `agent.rs`

Primary types:

- `Agent<M, S>`;
- `AgentTask`;
- `AgentCheckpoint`;
- `AgentRunError`;
- callback type aliases.

Responsibilities:

- build and restore an agent from settings or checkpoint;
- run initial actions exactly once;
- enforce max steps, max failures, step timeout, LLM timeout, stop, pause, and
  loop-detection guards;
- invoke the main, fallback, judge, and page-extraction LLMs;
- write conversation transcripts and GIF output;
- call step and done callbacks;
- compact message history;
- record final-response-after-failure and final-step responses.

Review risks:

- Changing run/step order can break upstream parity even when unit tests still
  pass.
- `done` handling is intentionally strict: final-step and failure responses
  must return exactly one `done` action.
- Fallback LLM switching is only for retryable provider errors.
- Checkpoint fields must stay serde-compatible.

### `prompt.rs`

Primary contracts:

- `build_step_request`;
- `build_step_request_with_file_system`;
- final-response, judge, and compaction request builders;
- agent output JSON schemas.

Responsibilities:

- render system/user prompt sections;
- include current browser state, history, read-state content, files, examples,
  screenshots, recent events, loop nudges, and budget warnings;
- apply sensitive-data placeholders and TOTP values;
- shorten prompt URLs and help restore model outputs;
- filter excluded actions from schemas without blocking `done`;
- scale screenshot coordinates back to viewport coordinates before execution.

Review risks:

- Prompt text is behavior. Keep changes fixture-backed.
- Schema changes must be paired with parser/runtime checks.
- Sensitive values should be substituted for execution but not leaked into
  history or prompt text.
- Large extracted content should move into read-state/file sections instead of
  repeatedly bloating previous-results text.

### `history.rs`

Responsibilities:

- store model output, action results, browser state, timing metadata, and usage;
- provide final-result, errors, screenshots, and judgement helpers;
- build replay plans that rematch historical DOM elements against current DOM;
- preserve compacted memory across message compaction.

Review risks:

- Replay matching is a contract with `browser-use-dom`; do not weaken exact or
  stable matches without a regression fixture.
- History JSON shape is checkpoint and conformance surface.

### `file_system.rs`

Responsibilities:

- create temp or configured managed sandboxes;
- serialize file-system state into checkpoints;
- resolve relative paths safely;
- read/write/append text, PDF, DOCX, and image artifacts;
- render file references in `done` results when settings allow it.

Review risks:

- Do not allow traversal out of the managed sandbox for relative paths.
- Upload availability checks should happen before browser side effects.

### `settings.rs`, `urls.rs`, `usage.rs`

`settings.rs` owns defaults and validation. `urls.rs` owns task URL extraction,
search URL construction, prompt URL shortening, and output restoration.
`usage.rs` owns token/cost accounting.

Review risks:

- Defaults are upstream compatibility claims.
- URL shortening must not rewrite image URLs or non-user/assistant content.
- Usage summaries must tolerate providers that omit partial usage fields.

### `executor.rs`

`executor.rs` owns browser-action execution:

- `ActionExecutor`;
- `BrowserActionExecutor`;
- `execute_action_sequence`;
- `execute_history_replay_plan`;
- extract/page text preparation helpers;
- browser action mapping into the `BrowserSession` trait.

Review risks:

- Multi-action execution stops after `done`, errors, navigation, and other
  sequence-terminating actions.
- Page-extraction LLM results should become `ActionResult` values, not agent
  control-flow errors.

### `lib.rs`

`lib.rs` declares modules, re-exports the public core API, and keeps
crate-level compatibility shims for tests and sibling modules. New behavior
should almost always live in one of the focused modules above.

## CDP Crate

Path: `crates/browser-use-cdp/src`.

### `lib.rs`

Primary public surface:

- `BrowserProfile`;
- `CdpBrowserSession`;
- `BrowserSession`;
- `BrowserLifecycleEvent` and adapter events;
- `BrowserError`;
- Cloud session request/response/client types.

Responsibilities:

- direct CDP session creation and attach;
- browser action methods for the `BrowserSession` trait;
- root public re-exports for split modules.

Review risks:

- `CdpBrowserSession` owns live target state. Move pure profile, storage,
  recording, DOM, transport, and watchdog behavior into the focused modules.
- `BrowserSession` methods are consumed by `browser-use-core`; errors and
  side effects should stay action-shaped.

### `cloud.rs`

Responsibilities:

- Browser Use Cloud create/stop requests;
- API-key resolution from explicit settings, env, and auth config;
- cloud HTTP headers and customer-facing error messages.

Review risks:

- Keep provider credentials out of serialized profile or tool input.
- Preserve upstream request aliases and tri-state proxy-country behavior.

### `profile.rs`

Responsibilities:

- `BrowserProfile` defaults, serde aliases, and validation;
- Chrome executable discovery and channel candidates;
- launch-plan argument construction and deduplication;
- local browser process lifecycle and `DevToolsActivePort` parsing.

Review risks:

- Profile serde aliases mirror upstream names. Preserve canonical output while
  accepting upstream aliases.
- Launch argument ordering is tested because it affects upstream parity.

### `lifecycle.rs`

Responsibilities:

- lifecycle event DTOs;
- adapter-event taxonomy for upstream-shaped consumers;
- subscription lag/closed semantics.

Review risks:

- Event names are integration surface. Add variants deliberately and test
  adapter mapping.

### `recording.rs`

Responsibilities:

- HAR event collection and file writing;
- trace artifact generation;
- video/GIF recording from screencast frames;
- artifact-path deduplication and diagnostics.

Review risks:

- Artifact recorders should report diagnostics without breaking normal browser
  use when optional dependencies fail.
- Avoid leaking trace artifact paths into lifecycle JSON except through explicit
  metadata fields.

### `storage.rs`

Responsibilities:

- cookie and origin storage-state capture;
- frame-origin discovery;
- DOMStorage conversion;
- storage-state load/apply/write helpers.

Review risks:

- Treat storage-state shape as public compatibility surface.
- Origin scripts must not run on the wrong origin.

### `transport.rs`

Responsibilities:

- connect to the CDP websocket;
- validate websocket headers;
- send JSON commands;
- route response ids to waiting callers;
- broadcast CDP events;
- reconnect with bounded delays;
- mark attached sessions stale after reconnect.

Review risks:

- The command actor must not hang pending requests silently.
- Reconnect events feed lifecycle diagnostics; keep event names stable.
- Session generation checks prevent stale target-session commands after a
  reconnect.

### `dom.rs`

Responsibilities:

- DOM indexing JavaScript;
- cached element action JavaScript;
- dropdown and scroll-to-text scripts;
- element and coordinate highlight scripts;
- DOMSnapshot/accessibility-tree joins through temporary AX refs;
- compact DOM, eval tree, page stats, and bounds parsing;
- iframe target offset and state merging;
- target-local index fallback;
- pagination button detection.

Review risks:

- The compact DOM state is a model prompt surface.
- Accessibility enrichment should add useful model cues without dumping raw AX
  trees.
- Cached-node fallback must stay target-aware for OOPIF iframe support.
- JavaScript strings should stay covered by focused tests because Rust type
  checking cannot see inside them.

### `watchdog.rs`

Responsibilities:

- lifecycle event watchdog;
- security URL-policy watchdog;
- bounded lifecycle and security buffers;
- websocket lifecycle event mapping;
- target crash and JavaScript dialog diagnostics;
- network request timeout tracking;
- browser download event mapping;
- auto-PDF response-body download;
- safe download filename and dedupe helpers.

Review risks:

- Watchdog tasks must abort when their owning session is dropped.
- Security watchdog actions should update pending URL-policy errors exactly
  once per blocked event.
- Diagnostics should be visible through lifecycle APIs and selected browser
  state fields, not normal agent replies.
- Download paths must remain contained in the configured or session-owned
  downloads directory.

## DOM Crate

Path: `crates/browser-use-dom/src/lib.rs`.

Key types:

- `SerializedDomState`;
- `DomElementRef`;
- `DomEvalNode`;
- `DomInteractedElement`;
- `BrowserStateSummary`;
- `PageInfo`;
- `TabInfo`;
- `PaginationButton`.

Responsibilities:

- define compact state DTOs shared by CDP and core;
- render DOM elements for prompt/history text;
- preserve interacted element metadata for replay/rematch;
- categorize rematch failures and levels.

Review risks:

- This crate is a shared serialization boundary. Add fields with serde defaults
  when older checkpoints or fixtures must continue to load.
- Prompt rendering changes ripple into agent conformance tests.

## Tools Crate

Path: `crates/browser-use-tools/src/lib.rs`.

Responsibilities:

- define `BrowserAction`;
- define action parameter structs;
- provide action names, schema derivations, and sequence-termination metadata.

Review risks:

- Renaming variants or fields changes model output schemas.
- New actions require coordinated updates in core prompt schemas, action
  execution, CDP session methods, CLI/MCP exposure if user-facing, and tests.

## LLM Crate

Path: `crates/browser-use-llm/src/lib.rs`.

Responsibilities:

- define `ChatModel`, `ChatRequest`, `ChatMessage`, `ContentPart`,
  `ChatCompletion`, and `ChatUsage`;
- implement provider adapters and structured-output fallback behavior;
- normalize model-specific wire shapes into the shared chat contract.

Review risks:

- Provider-specific hacks should not leak into core.
- If a provider cannot enforce JSON schema directly, keep fallback prompt text
  and response parsing tested.
- Image detail and content-part ordering affect prompt parity.

## CLI Crate

Path: `crates/browser-use-cli/src/main.rs`.

Responsibilities:

- parse CLI commands;
- launch one-shot browser actions;
- run agent tasks;
- manage persistent local sessions;
- run local daemon/TCP JSON-RPC surfaces;
- print stable JSON responses.

Review risks:

- CLI JSON is a user contract. Avoid casual field churn.
- Persistent session state must be cleaned up explicitly.
- CLI code should compose core/CDP APIs rather than duplicating browser logic.

## MCP Crate

Path: `crates/browser-use-mcp/src/lib.rs`.

Responsibilities:

- expose MCP tools backed by CLI/core/session behavior;
- publish typed schemas for tool arguments and outputs;
- keep MCP result shapes aligned with CLI/session contracts.

Review risks:

- MCP errors should be actionable but not leak local implementation details.
- Tool schema changes should be treated like public API changes.

## Conformance Crate And Fixtures

Path: `crates/browser-use-conformance/src/lib.rs` plus `fixtures/`.

Responsibilities:

- load fixture data;
- normalize golden outputs;
- make compatibility tests cheap to write and review.

Review risks:

- A fixture update should explain the behavior change it accepts.
- Do not update golden files to hide regressions.

## Common Change Recipes

### Add A Browser Action

1. Add the action DTO and enum variant in `browser-use-tools`.
2. Add prompt schema support in `browser-use-core/src/prompt.rs`.
3. Add execution mapping in `browser-use-core/src/executor.rs`.
4. Add a `BrowserSession` trait method if the action needs browser side
   effects.
5. Implement the CDP behavior in the narrowest supporting module, usually
   `dom.rs`, `profile.rs`, `recording.rs`, `storage.rs`, `watchdog.rs`, or the
   session methods in `lib.rs`.
6. Add unit tests for schema, prompt rendering, execution, and CDP behavior.
7. Add CLI/MCP surfaces only if the action is user-facing there.

### Add A Browser State Field

1. Add the DTO field in `browser-use-dom` with serde defaults if needed.
2. Parse it in `browser-use-cdp/src/dom.rs`.
3. Render it in DOM text or prompt sections only when it helps action choice.
4. Add fixtures and prompt tests.
5. Verify checkpoints, CLI JSON, and MCP JSON if exposed.

### Add A Provider Quirk

1. Keep the branch inside `browser-use-llm`.
2. Normalize back into `ChatCompletion`.
3. Add provider-focused tests.
4. Avoid conditionals in `Agent`.

### Add A Lifecycle Diagnostic

1. Add or reuse a `BrowserLifecycleEventKind`.
2. Emit from `browser-use-cdp/src/watchdog.rs` or the session method that owns
   the event.
3. Keep bounded buffers intact.
4. Add JSON shape tests and, when live behavior matters, an ignored CDP test.

## Review Checklist

- Does the change preserve the frozen upstream compatibility claim?
- Is the owner module the right place for the behavior?
- Are public re-exports stable?
- Are serde defaults/aliases considered?
- Are browser side effects guarded before execution when possible?
- Are long-running tasks owned and abortable?
- Are prompt changes fixture-backed?
- Are live Chrome behaviors covered by ignored CDP tests when unit tests cannot
  prove the contract?
