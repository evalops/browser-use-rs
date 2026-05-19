# Conformance Plan

The project is successful only when behavior is demonstrably compatible with
browser-use where compatibility is claimed.

## Inputs

- Upstream source at `157779338afdcc03023010ec3c24ad63d820453c`.
- Upstream docs for quickstart, CLI, browser configuration, custom tools, and
  supported models.
- Upstream test intent from `tests/ci` and task fixtures.
- Local deterministic HTML fixtures for browser, DOM, and action behavior.

## Test Families

1. Schema snapshots: action JSON schemas, browser state JSON, and agent output.
2. DOM fixtures: numbered clickable elements, text representation, configurable
   prompt attributes, selector maps, iframes, hidden elements, dropdowns, ARIA
   widget roles, accessibility names/descriptions, compact AX metadata, and
   eval/judge DOM tree output.
3. Browser actions: navigation, URL access policy guards for explicit
   navigation, action boundaries, redirects, non-standard IPv4 blocking, and
   new tabs, search, click, input, scroll, keyboard, tab switching, downloads
   with sanitized page-controlled filenames, screenshots, PDF output,
   and cached-node fallback inside same-page and Chrome OOPIF iframe targets.
4. Agent loop: max steps, max failures, multi-action aborts after navigation,
   loop nudges, planning fields, prompt attribute settings, extraction
   metadata, per-step timing metadata, excluded action schema/runtime controls,
   recent-event prompt controls, upstream-compatible auto vision modes,
   screenshot action gating, vision detail levels, `done` file-display controls,
   conversation transcript saving, non-fatal judge trace validation, done
   semantics, and final history.
5. Provider contracts: OpenAI-compatible Chat Completions, OpenAI-wire upstream
   aliases, Anthropic, Gemini, and Ollama structured-output payloads first,
   including DeepSeek forced tool-call guidance and Cerebras prompt-only
   guidance, then deeper provider-specific fallback paths as compatibility
   expands.
6. CLI/MCP: persistent session lifecycle, JSON output stability, and error
   shapes.
7. Browser/profile lifecycle: public lifecycle event JSON shape, bounded event
   buffers, and security-watchdog diagnostics for target/page transitions.

## Accessibility Snapshot Boundary

The Rust port keeps accessibility data in the compact numbered DOM contract
rather than exposing raw full AX snapshots to the model. Indexed elements are
joined to Chrome `Accessibility.getFullAXTree` nodes through temporary DOM
markers and backend node ids. The supported AX surface now includes role, name,
description, common state/value properties, top-level AX `value` and
`description` fields, compact `ax_name`/`ax_description` metadata,
backend/frontend node ids, and the upstream clickability veto for AX
`hidden=true` or `disabled=true`.

Prompt rendering stays intentionally small. Default DOM text includes
automation-relevant aliases such as `expanded`, `pressed`, `selected`,
`keyshortcuts`, `valuemin`, `valuemax`, `valuenow`, `valuetext`,
live-region metadata, hierarchy metadata, multiselect metadata, compact
`ax_description`, and form values where safe. Longer or lower-frequency
metadata such as raw AX `description`, `focusable`, `editable`, and `settable`
is preserved in selector-map attributes and can be rendered through
`include_attributes`, but is not emitted by default. Password fields continue
to suppress `value` and `valuetext`.

`BrowserProfile.paint_order_filtering` defaults to upstream's `true` and gates
the DOM snapshot topmost-center `elementFromPoint` occlusion veto. Disabling it
keeps normal hidden, disabled, layout-size, and CSS visibility checks while
allowing visually covered elements to remain in the selector map for debugging
or conformance captures.

### DOM/AX Parity Checklist

The frozen upstream DOM/AX audit covered `browser_use/dom/views.py`,
`browser_use/dom/serializer/serializer.py`,
`browser_use/dom/serializer/eval_serializer.py`,
`browser_use/dom/serializer/clickable_elements.py`, and
`browser_use/dom/serializer/paint_order.py`.

Action-relevant parity is implemented for:

- Indexed selector maps and compact LLM/eval tree rendering instead of raw CDP
  dumps.
- DOM attributes used by upstream for action selection: form values, ids/names,
  roles, placeholders, labels, input masks, autocomplete, date-format hints,
  validation attributes, rich-text/editable hints, file-accept metadata, ARIA
  state/value aliases, and key shortcuts.
- Interactive detection from native controls, ARIA roles, JS click/pointer
  listeners where Chrome exposes them, labels/wrappers around form controls,
  search affordances, small icon controls, scroll containers, and media or
  compound controls.
- Visibility and pruning behavior: hidden/disabled/AX-hidden vetoes,
  decorative SVG child suppression, non-content DOM tag suppression,
  contained-action descendant pruning, long duplicate attribute pruning,
  configurable paint-order occlusion filtering, and bounded lifecycle-safe
  diagnostics rather than prompt metadata.
- Frame and shadow behavior: same-origin iframe traversal, attached OOPIF
  target content, open shadow-root controls, backend/frontend node identity,
  and merged target-aware cached-node action fallback.
- AX enrichment needed by the model and evaluator: role, name, description,
  top-level value/description, common state/value properties, quiet
  focusable/editable/settable metadata, `ax_name`/`ax_description`, and
  action-history rematching via exact/stable hash, XPath, AX name, and unique
  attributes.

Intentional non-goals are raw full AX object graphs in normal prompts or
state/action replies: AX child-id relationship graphs, ignored-reason trees,
related-node source chains, and full unfiltered CDP `Accessibility` or
`DOMSnapshot` payloads. Those are kept out unless a future fixture proves they
improve action selection, evaluation, or deterministic replay without bloating
the compact browser-use prompt contract.

Fixture coverage lives in `mixed_interactive_state.json`,
`frame_shadow_state.json`, `eval_tree_state.txt`, browser lifecycle fixtures,
and focused CDP/browser tests for accessibility-tree enrichment, ARIA widgets,
labels, selected options, media controls, hidden file inputs, iframe targets,
shadow DOM, JS listener controls, occlusion filtering, scroll containers, and
cached-node fallback.

## Browser/Profile Lifecycle Audit

The frozen upstream target exposes a broad browser event bus with
`BrowserStartEvent`, `BrowserStopEvent`, `BrowserConnectedEvent`,
`BrowserStoppedEvent`, `TabCreatedEvent`, `TabClosedEvent`,
`AgentFocusChangedEvent`, `TargetCrashedEvent`, `NavigationStartedEvent`,
`NavigationCompleteEvent`, `BrowserErrorEvent`, reconnection events, storage
state events, download events, JavaScript dialog handling, and browser
permission grants through watchdogs.

The Rust port currently exposes bounded `BrowserLifecycleEvent` diagnostics for
browser connect/close requests, target create/switch/close/crash, navigation
start/complete/failure/timeout, URL-policy navigation blocks, current-tab
resets, popup closes and popup close/reset failures, browser reconnects,
JavaScript dialog handling, download start/progress/completion, storage-state
save/load notifications, and browser diagnostics for non-fatal permission-grant
failures. These lifecycle events are available
through the CDP session API and are intentionally kept out of normal agent
replies; prompt state still only includes security-relevant recent events and
closed-popup messages. The public JSON shape is locked by normal and
exceptional lifecycle fixtures. Live CDP wiring records target crash,
JavaScript dialog, navigation failure, configured download events, and cookie
plus attached frame-tree origin storage-state save/load events. Root
`Browser.grantPermissions` failures record browser diagnostics without failing
startup. `BrowserProfile.headers` are scoped to CDP websocket connection
handshakes and reconnect handshakes; they are not injected as page request
headers. CDP websocket closure records a browser-stopped lifecycle diagnostic, and direct
`Page.navigate` timeouts plus stuck HTTP(S) requests record network-timeout
lifecycle diagnostics. Direct PDF viewer URLs are marked as PDF state and, when
`BrowserProfile.accept_downloads` and `auto_download_pdfs` remain enabled, are
downloaded once per session with safe filenames and `auto_download=true`
lifecycle metadata. Accepted sessions use an explicit `downloads_path` or a
session-owned temporary directory; upstream `downloads_dir` and
`save_downloads_path` profile aliases resolve into the same canonical
`downloads_path` field before download behavior is configured.
`accept_downloads=false` skips CDP download setup and PDF auto-download writes
even if `downloads_path` is configured. The
direct-CDP path first uses `Network.responseReceived` metadata and
`Network.getResponseBody` bytes when Chrome exposes the PDF response, including
content-disposition filenames, then falls back to the conservative direct-URL
path. Explicit `auto_download_pdfs=false` preserves normal download events but
skips that PDF auto-download path.
`BrowserProfile.record_har_content` defaults to `embed`,
`record_har_mode` defaults to `full`, and `record_har_path` defaults to unset.
`save_har_path` deserializes into the canonical `record_har_path` field. When
configured, the direct-CDP session records HTTPS request/response/loading data
into a HAR 1.2 file on best-effort `close_browser()` flush. `minimal` mode
keeps entries for the main page origin, `full` keeps captured HTTPS entries
except favicon requests, and `omit`/`embed`/`attach` control response and
request body representation.
`BrowserProfile.record_video_dir` defaults to unset and accepts upstream
`save_recording_path`; canonical JSON emits `record_video_dir`.
`record_video_size` uses the same viewport-size shape, and
`record_video_framerate` defaults to `30`. `record_video_format` defaults to
`mp4`, also accepts `.webm`/`webm` and `.gif`/`gif`, and serializes only when
overriding the MP4 default. When configured, direct-CDP sessions start PNG
screencast capture for the focused target, acknowledge captured frames, switch
screencast sessions when the focused target changes, and write a close-time
recording under `record_video_dir`. MP4 and WebM output use a runtime `ffmpeg`
encoder with codec-compatible padded frames; if that encoder is unavailable or
fails, the Rust port records an encode diagnostic and preserves a
dependency-light GIF fallback. Video start/stop/encode failures record browser
diagnostics rather than failing normal startup/close, and video artifact paths
stay out of normal browser state, action, and agent replies.
`BrowserProfile.traces_dir` defaults to unset and accepts upstream
`trace_path`; canonical JSON emits `traces_dir`. At the frozen upstream target,
`traces_dir` is a profile field described as a Playwright trace zip directory,
but no `browser_use/browser` trace watchdog or Playwright tracing runtime is
wired. The Rust direct-CDP boundary is therefore explicit: configured sessions
write a best-effort close-time JSON artifact with schema
`browser-use-rs.trace.v1`, artifact kind `browser-use-rs.cdp_json_trace`,
`runtime="direct_cdp"`, and `playwright_trace_zip=false`. The artifact captures
lifecycle events, security diagnostics, current target ids, and the last cached
DOM state. Trace artifact write failures record a browser diagnostic with phase
`write` and still allow normal browser close to proceed. Trace artifact paths,
artifact kind, and trace metadata are not added to normal browser state,
action, or agent replies.
Unexpected websocket drops trigger bounded actor-level attempts with
reconnecting/reconnected/failure lifecycle diagnostics. Registered CDP target
sessions are invalidated after reconnect so stale session-scoped commands fail
locally with a clear reattach error, and the current target is reattached
automatically on the next session access when Chrome still exposes it.

The bounded history and `subscribe_lifecycle_events` subscription facade are
both kept out of normal agent replies unless an integration explicitly reads
them. The facade exposes `recv`/`try_recv` with typed lag and closed-stream
errors so downstream integrations do not need to depend on Tokio broadcast
receiver details. `BrowserLifecycleAdapterEvent` maps those diagnostics into
upstream-style subscriber concepts such as tab created/closed, agent focus
changed, navigation started/complete, browser errors, storage state, downloads,
dialogs, reconnects, and browser diagnostics; its JSON shape is locked by a
conformance fixture.

## Viewport Emulation Boundary

`BrowserProfile.channel` preserves the upstream browser-channel strings
(`chromium`, `chrome`, `chrome-beta`, `chrome-dev`, `chrome-canary`,
`msedge`, `msedge-beta`, `msedge-dev`, and `msedge-canary`) and constrains the
local executable search to channel-specific candidates when no explicit
`executable_path` or `BROWSER_USE_CHROME` override is supplied. Explicit paths
and the environment override retain precedence so existing local deployments do
not change accidentally. Upstream `browser_binary_path` and
`chrome_binary_path` profile aliases deserialize into the same canonical
`executable_path` field and serialize back as `executable_path`.

`BrowserProfile.screen`, `viewport`, `no_viewport`, and `device_scale_factor`
are preserved in the Rust profile contract. Launch planning uses `screen` as a
window-size fallback when no explicit `window_size` is set, rejects
`headless=true` with `no_viewport=true`, and applies
`Emulation.setDeviceMetricsOverride` with `mobile=false` whenever viewport mode
is active. The override is applied to the initial page, new tabs, explicit tab
switches, stale-session reattachments, and replacement pages selected after the
focused tab closes. `no_viewport=true` intentionally leaves page content to the
real browser window and skips the CDP override.

`BrowserProfile.window_position` defaults to upstream's `0,0` origin position
and is emitted in launch plans even when callers do not supply an override.
Explicit window positions still override the default before raw custom args are
de-duped with last-wins switch behavior.

## Interaction Highlight Boundary

`BrowserProfile.highlight_elements` defaults to `true`, with upstream-default
`interaction_highlight_color="rgb(255, 127, 39)"` and
`interaction_highlight_duration=1.0`. Indexed click/input actions attempt a
non-fatal temporary highlight when the cached DOM element has viewport bounds,
and coordinate clicks attempt a non-fatal temporary marker at the clicked
viewport coordinate. `highlight_elements=false` disables those injected
markers without changing the underlying action behavior.
`BrowserProfile.dom_highlight_elements` defaults to `false`; when enabled,
state capture attempts to remove the prior debug-highlight container and draw a
fresh overlay for the current selector map. `filter_highlight_ids=true`
preserves upstream-style quiet labels by suppressing ids for verbose element
representations, while `false` labels every highlighted element. Overlay
failures never fail state capture.

## Page-Load Wait Boundary

`BrowserProfile.minimum_wait_page_load_time` and
`wait_for_network_idle_page_load_time` are preserved with upstream-compatible
defaults of `0.25` and `0.5` seconds. The Rust CDP session applies the minimum
delay before browser-state capture and after successful navigation, then waits
for the shared CDP network-event tracker to observe the configured idle window.
Setting either value to `0` disables that part of the settle path. The wait is
bounded by the configured idle duration and does not expose per-request network
activity in normal agent replies.

## OOPIF Stale-Node Fallback Boundary

Cached observed-node actions first resolve the original backend/frontend node
inside its recorded CDP target session. If Chrome reports that cached node as
stale or detached, click, input, scroll, dropdown, and upload fallback
traversal runs in the cached element's target session rather than the current
main-frame session. Merged DOM indexes are translated back to the target-local
interactive index before running fallback JavaScript, so Chrome OOPIF iframe
actions do not drift onto main-frame elements after a child-frame node is
replaced.

The fallback remains bounded to attached page and iframe target sessions that
Chrome exposes through CDP. It does not use browser profile internals or unsafe
cross-origin DOM reads outside those target sessions.

`BrowserProfile.cross_origin_iframes`, `max_iframes`, and `max_iframe_depth`
are honored at this boundary. Same-origin iframe document traversal is capped
inside the injected DOM snapshot script. Chrome OOPIF target traversal keeps the
parent iframe element visible, can be disabled with `cross_origin_iframes=false`,
and caps direct iframe target fanout before attaching CDP sessions. The current
direct-CDP implementation does not recursively reconstruct offsets for nested
OOPIF target trees beyond attached page targets; deeper browser-profile
internals stay out of scope until Chrome exposes a safe offset source.

## Profile-Wide Storage Boundary

The supported storage-state boundary is intentionally the current page plus
attached frame-tree HTTP(S) origins. CDP `DOMStorage.getDOMStorageItems` works
from a caller-provided `StorageId` containing a `securityOrigin` or
`storageKey`, but it does not enumerate every local/session storage origin in a
browser profile. At the frozen upstream target, browser-use's CDP path uses
`Page.getFrameTree` for exactly this origin inventory before calling
`DOMStorage.getDOMStorageItems`; it does not scrape profile files or enumerate
unattached profile origins. The CDP `Storage` domain exposes cookies,
usage/quota by a caller-provided origin, and frame-derived storage keys, but not
a safe profile-wide localStorage/sessionStorage origin inventory. Profile-wide
storage discovery therefore remains an explicit non-goal unless a later Chrome
surface exposes it without navigating pages or reading browser profile
internals.

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
  compact accessibility names/descriptions, top-level AX values, raw opt-in AX
  descriptions, attributes, prompt-visible ARIA state aliases, quiet AX
  clickability metadata, bounds, dropdown current values, compound control
  metadata, scrollable metadata, rich-text editors, and media controls.
- `frame_shadow_state.json`: selector-map fixture for iframe target identity,
  merged child-frame bounds, and open-shadow-style indexed controls.
- `eval_tree_state.txt`: tree-shaped eval representation fixture covering
  structural tags, backend-node `[i_*]` markers, compact accessibility
  metadata, shadow-root markers, and iframe-content markers.
- `browser_action_schema.json`: JSON Schema snapshot for the implemented
  one-key browser action contract.
- `browser_state_summary_schema.json`: JSON Schema snapshot for serialized
  browser state returned to the agent loop.
- `agent_output_schema.json`: JSON Schema snapshot captured from the default
  agent model request, including required browser-use planning fields and
  non-empty action guidance.
- `agent_history_replay_run_schema.json`: JSON Schema snapshot for the public
  `AgentHistoryReplayRun` replay output contract returned by CLI and MCP replay
  surfaces.
- `rich_browser_state_summary.json`: top-level browser-state JSON fixture for
  DOM state, tabs, screenshot markers, page metrics, network activity,
  pagination affordances, browser errors, recent events, and popup closure
  messages.
- `agent_history_replay_run.json`: serialized `AgentHistoryReplayRun` fixture
  covering current-state capture, remapped replay plan items, interacted-element
  match diagnostics, guarded execution results, and replay stop reasons.
- `agent_history_replay_recapture_run.json`: serialized
  `AgentHistoryReplayRun` fixture covering browser-backed replay state
  recapture between actions and later indexed-action rematching against the
  latest DOM.
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
