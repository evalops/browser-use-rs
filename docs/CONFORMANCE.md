# Conformance Plan

The project is successful only when behavior is demonstrably compatible with
browser-use where compatibility is claimed.

## Inputs

- Upstream source at `933e28c599ddd74c15a48568f159da95547e40dd`.
- Upstream docs for quickstart, CLI, browser configuration, custom tools, and
  supported models.
- Upstream test intent from `tests/ci` and task fixtures.
- Local deterministic HTML fixtures for browser, DOM, and action behavior.

## Test Families

1. Schema snapshots: action JSON schemas, browser state JSON, and agent output.
2. DOM fixtures: numbered clickable elements, text representation, configurable
   prompt attributes, selector maps, iframes, hidden elements, dropdowns, ARIA
   widget roles, accessibility names, and eval/judge DOM tree output.
3. Browser actions: navigation, URL access policy guards for explicit
   navigation, action boundaries, redirects, and new tabs, search, click,
   input, scroll, keyboard, tab switching, downloads, screenshots, and PDF
   output.
4. Agent loop: max steps, max failures, multi-action aborts after navigation,
   loop nudges, planning fields, prompt attribute settings, extraction
   metadata, per-step timing metadata, done semantics, and final history.
5. Provider contracts: OpenAI-compatible Chat Completions, OpenAI-wire upstream
   aliases, Anthropic, Gemini, and Ollama structured-output payloads first,
   including DeepSeek forced tool-call guidance and Cerebras prompt-only
   guidance, then deeper provider-specific fallback paths as compatibility
   expands.
6. CLI/MCP: persistent session lifecycle, JSON output stability, and error
   shapes.
7. Browser/profile lifecycle: public lifecycle event JSON shape, bounded event
   buffers, and security-watchdog diagnostics for target/page transitions.

## Browser/Profile Lifecycle Audit

The frozen upstream target exposes a broad browser event bus with
`BrowserStartEvent`, `BrowserStopEvent`, `BrowserConnectedEvent`,
`BrowserStoppedEvent`, `TabCreatedEvent`, `TabClosedEvent`,
`AgentFocusChangedEvent`, `TargetCrashedEvent`, `NavigationStartedEvent`,
`NavigationCompleteEvent`, `BrowserErrorEvent`, reconnection events, storage
state events, download events, and JavaScript dialog handling through watchdogs.

The Rust port currently exposes bounded `BrowserLifecycleEvent` diagnostics for
browser connect/close requests, target create/switch/close/crash, navigation
start/complete/failure/timeout, URL-policy navigation blocks, current-tab
resets, popup closes and popup close/reset failures, browser reconnects,
JavaScript dialog handling, download start/progress/completion, and
storage-state save/load notifications. These lifecycle events are available
through the CDP session API and are intentionally kept out of normal agent
replies; prompt state still only includes security-relevant recent events and
closed-popup messages. The public JSON shape is locked by normal and
exceptional lifecycle fixtures. Live CDP wiring records target crash,
JavaScript dialog, navigation failure, configured download events, and cookie
plus attached frame-tree origin storage-state save/load events. CDP websocket
closure records a browser-stopped lifecycle diagnostic, and direct
`Page.navigate` timeouts plus stuck HTTP(S) requests record network-timeout
lifecycle diagnostics. Unexpected websocket drops trigger bounded actor-level
attempts with reconnecting/reconnected/failure lifecycle diagnostics. Registered
CDP target sessions are invalidated after reconnect so stale session-scoped
commands fail locally with a clear reattach error, and the current target is
reattached automatically on the next session access when Chrome still exposes
it.

The bounded history and `subscribe_lifecycle_events` subscription facade are
both kept out of normal agent replies unless an integration explicitly reads
them. The facade exposes `recv`/`try_recv` with typed lag and closed-stream
errors so downstream integrations do not need to depend on Tokio broadcast
receiver details. `BrowserLifecycleAdapterEvent` maps those diagnostics into
upstream-style subscriber concepts such as tab created/closed, agent focus
changed, navigation started/complete, browser errors, storage state, downloads,
dialogs, reconnects, and browser diagnostics; its JSON shape is locked by a
conformance fixture.

## Profile-Wide Storage Boundary

The supported storage-state boundary is intentionally the current page plus
attached frame-tree HTTP(S) origins. CDP `DOMStorage.getDOMStorageItems` works
from a caller-provided `StorageId` containing a `securityOrigin` or
`storageKey`, but it does not enumerate every local/session storage origin in a
browser profile. The CDP `Storage` domain exposes cookies, usage/quota by a
caller-provided origin, and frame-derived storage keys, but not a safe
profile-wide localStorage/sessionStorage origin inventory. Profile-wide storage
discovery therefore remains out of the supported boundary unless a later Chrome
surface exposes it without navigating pages or reading browser profile internals.

References:

- https://chromedevtools.github.io/devtools-protocol/tot/DOMStorage/
- https://chromedevtools.github.io/devtools-protocol/tot/Storage/

## Drift Policy

Upstream bumps must include:

- Old and new upstream commit SHAs.
- A summary of changed contracts.
- Updated conformance fixtures or explicit deferred gaps.
- A changelog entry describing compatibility impact.

## Current Fixtures

- `simple_interactive_state.json`: compact DOM text and selector-map fixture.
- `mixed_interactive_state.json`: selector-map fixture for accessible labels,
  attributes, prompt-visible ARIA state aliases, bounds, dropdown current
  values, compound control metadata, scrollable metadata, rich-text editors, and
  media controls.
- `frame_shadow_state.json`: selector-map fixture for iframe target identity,
  merged child-frame bounds, and open-shadow-style indexed controls.
- `eval_tree_state.txt`: tree-shaped eval representation fixture covering
  structural tags, backend-node `[i_*]` markers, shadow-root markers, and
  iframe-content markers.
- `browser_action_schema.json`: JSON Schema snapshot for the implemented
  one-key browser action contract.
- `browser_state_summary_schema.json`: JSON Schema snapshot for serialized
  browser state returned to the agent loop.
- `agent_output_schema.json`: JSON Schema snapshot captured from the default
  agent model request, including required browser-use planning fields and
  non-empty action guidance.
- `rich_browser_state_summary.json`: top-level browser-state JSON fixture for
  DOM state, tabs, screenshot markers, page metrics, network activity,
  pagination affordances, browser errors, recent events, and popup closure
  messages.
- `simple_action_sequence.json`: typed browser action sequence fixture.
- `simple_action_results.json`: expected action-result fixture for the action
  sequence harness.
- `simple_agent_history.json`: deterministic scripted-agent replay fixture
  covering schema-guided model output, previous-result prompt context, browser
  action execution, `done`, and serialized history. Runtime step timing
  metadata is asserted semantically before dynamic timestamps are stripped for
  golden comparison.
- `long_task_agent_replay.json`: deterministic longer scripted-agent replay
  fixture covering planning guidance, revised plans after a failed browser
  action, prompt-history limits, stagnant-page loop-awareness, managed report
  file write/read/display behavior, and final `done`.
- `managed_file_system_replay.json`: deterministic managed `FileSystemState`
  replay fixture covering normalized serialized filesystem state, restored
  agent prompt context for `<file_system>` and `<todo_contents>`, restored
  `read_file` one-time read-state replay, and extracted-content numbering that
  continues after restore.
- `agent_checkpoint_replay.json`: deterministic full-agent checkpoint fixture
  covering normalized managed filesystem state inside the checkpoint,
  serialized initial actions, prior history/action results, resumed
  `<file_system>` and `<todo_contents>` prompt context, restored read-state
  replay, and extracted-content numbering after checkpoint resume.
- `browser_lifecycle_events.json`: public lifecycle event JSON fixture covering
  browser connection, target creation, navigation completion, and URL-policy
  popup failure diagnostics.
- `browser_lifecycle_adapter_events.json`: public upstream-style lifecycle
  adapter JSON fixture covering tab, focus, navigation, security diagnostic,
  browser error, and storage-state mappings.
