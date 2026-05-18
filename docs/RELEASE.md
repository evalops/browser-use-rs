# Release Support Matrix

This release targets:

```text
browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd
```

## Supported

- Local Chrome/Chromium launch and CDP attach.
- Browser profile URL access policies for explicit navigation, including
  allowed/prohibited domain patterns, allowed-domain precedence, internal
  browser URL allowances, data/blob URL allowances, authentication-bypass
  resistance, and optional IP-address blocking, plus post-navigation redirect
  checks, blocked-navigation preflight diagnostics, navigation-capable
  action-boundary checks, newly observed tab closure for disallowed URLs, and
  event-driven target/frame navigation watchdog enforcement while a session is
  active. CDP sessions expose bounded `BrowserLifecycleEvent` diagnostics for
  browser connect/close, target create/switch/close, navigation start/complete,
  navigation failure/timeout, target crash, URL-policy block/reset/popup
  outcomes, reconnect, JavaScript dialog, download, and storage-state event
  shapes. `BrowserProfile.downloads_path` enables browser download behavior and
  CDP download lifecycle events for launched sessions; `storage_state_path`
  loads and saves browser cookie plus attached frame-tree origin local/session
  storage state with storage lifecycle events.
  CDP websocket closure records a browser-stopped lifecycle diagnostic, and
  unexpected websocket drops trigger bounded actor-level reconnect attempts
  with reconnecting/reconnected/failure lifecycle diagnostics. Registered CDP
  target sessions are invalidated after reconnect so stale session-scoped
  commands fail locally with a clear reattach error, and the current target is
  reattached automatically on the next session access when Chrome still exposes
  it.
  `BrowserProfile.navigation_timeout_ms` bounds direct `Page.navigate` calls
  and records network-timeout lifecycle diagnostics on timeout.
  `network_request_timeout_ms` records lifecycle diagnostics for HTTP(S)
  requests that remain active beyond the watchdog budget.
- Browser state with URL, title, tabs plus browser-use-style short tab ids,
  screenshots, page metrics, compact DOM state, element bounds, open
  shadow-root indexing, same-origin iframe tag and content indexing, scrollable
  element metadata, Chrome OOPIF cross-origin iframe target content indexing
  and cached-node actions, automation-friendly data/ARIA/value attributes,
  native boolean/read-only state, validation patterns, `data-state`, static
  history-matching attributes, accessibility-tree role/name/state/value
  enrichment with backend/frontend node ids, hidden-element and
  `data-browser-use-exclude` subtree filtering, topmost/occlusion filtering,
  hidden file-input upload targets, plain scroll-container indexing,
  non-content tag pruning, prompt-visible pages-above/below context for indexed
  scroll containers, href-less anchor tags, accessible names from labels, ARIA
  references, image alt text, selected dropdown values, compound control
  metadata, compact select option summaries, common ARIA widget roles, search
  affordance signals, small icon controls, tabindex-backed controls including
  `tabindex="-1"`, ARIA required/autocomplete/keyshortcut interactivity signals
  with prompt-visible `keyshortcuts`, AX-shaped numeric value aliases,
  human-readable value text, contenteditable editor variants, media control
  compounds, duplicate long-attribute pruning,
  input mask/autocomplete/date-format datepicker hints, live-region, hierarchy,
  and multiselect state aliases, JavaScript click/pointer listener-backed
  controls, cursor-pointer controls, decorative SVG child pruning, static
  mouse/keyboard handler attributes, contained duplicate-descendant pruning for
  action containers, pagination affordance detection, configurable
  prompt-visible attributes, the upstream empty-DOM load hint, and a
  CDP-populated tree-shaped eval/judge DOM representation with backend-node
  interactive markers, shadow-root markers, iframe-content markers, compact key
  attributes, scroll context, and collapsed SVG contents.
- Built-in actions for search, navigate, back navigation, 4-character tab-id
  switch/close, click, coordinate click, input, page or indexed element scroll,
  wait, text-target scroll, browser JavaScript evaluation, screenshot, native and
  ARIA dropdown options/selection, keyboard text/special-key/shortcut events,
  file upload, local text-file read/write/replace with upstream-style CSV row
  normalization and relative filename sanitization, page-aware PDF read
  envelopes, PDF/DOCX write/append artifacts with paginated PDF text layout,
  and append-only-to-existing-file semantics, upstream-aligned binary/image
  extension rejection, DOCX text extraction, PNG/JPEG image-file reads with
  one-shot image prompt parts, PDF capture, extraction, page search, element
  lookup across Chrome OOPIF iframe targets, cached observed-node
  click/input/scroll/dropdown/upload resolution, and done.
- `screenshot` requests screenshot inclusion in the next observation by default
  and writes a local `.png` file with an attachment path when `file_name` is
  supplied.
- `save_as_pdf` writes a local PDF file, appends `.pdf` when missing, derives a
  safe page-title filename when omitted, avoids overwriting existing files, and
  returns the saved file as an attachment.
- `done.files_to_display` appends readable requested text files to the final
  result and returns their attachment paths.
- Managed `FileSystem` state with a `browseruse_agent_data` sandbox directory,
  default `todo.md`, file listing/display, extract-content numbering,
  serialization/restoration, nuke, and disk sync for text, CSV, PDF, and DOCX
  artifacts. Executor-owned relative file actions and `done.files_to_display`
  route through that sandbox while absolute external paths bypass it. Agent
  prompts include upstream-style `<file_system>` and `<todo_contents>` context,
  and large extract results can spill into managed `extracted_content_N.md`
  files. Restored agents can continue from serialized `FileSystemState` with
  prompt-visible todo/file context, restored `read_file` behavior, and
  extracted-content numbering that survives replay.
- Browser-aware action sequencing that stops on errors, done, explicit
  terminating actions, and URL changes after browser actions.
- Agent runs with schema-guided provider output, upstream-style initial actions,
  max actions per step with upstream-style truncation, max steps, max failures,
  step and LLM timeouts, upstream-style final `done` responses after repeated
  failures, normalized repeated-action loop detection, previous result context,
  vision-aware screenshot capture and image prompt parts, screenshot action
  next-observation image overrides, action-result image prompt parts,
  upstream-style page-stat prompt context with loading/skeleton hints, one-time
  extraction replay handling, invalid model-output recovery, loop-awareness
  prompt nudges, upstream
  flattened planning fields, configurable planning prompt nudges, per-step
  timing metadata, thinking/flash output-schema controls, upstream-style
  flattened required output fields, upstream-style prompt-history inclusion and
  limits, clickable-element text limits, upstream-style one-time read-state
  prompt blocks, upstream-style tagged agent-history/agent-state/browser-state
  prompt sections, upstream-style available-file-path and sensitive-data
  placeholder context with `bu_2fa_code` TOTP generation, system-message
  override/extension controls, upstream-style prompt context/error truncation,
  typed upstream-style last-result completion helpers, upstream-compatible
  action-result success validation, judgement results, and step-error,
  model-output, model-action, thought, duration, truncated action-history, and
  screenshot/URL accessors. `AgentCheckpoint` export/resume preserves task
  settings, history, initial-action execution state, and managed filesystem
  state across a new model/session.
- Schema-guided extraction results include structured metadata with schema,
  partial status, content statistics, link/image counts, and de-duplication
  counts.
- Scripted agent replay conformance fixtures for schema-guided model output,
  previous-result prompt context, action execution, `done`, serialized
  history, longer multi-step planning/recovery replay with prompt-history
  limits and stagnant-page loop-awareness, managed `FileSystemState` replay
  through restored prompts, `read_file`, todo context, extracted-content
  numbering, and full `AgentCheckpoint` resume with prior history and
  initial-action state, plus public browser lifecycle event and adapter JSON
  shapes, with semantic checks for dynamic step timing metadata.
- OpenAI-compatible Chat Completions plus DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, and Vercel AI Gateway aliases, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output requests,
  including DeepSeek forced tool-call, Cerebras prompt-only, and OpenAI-wire
  output-mode override payload/parser modes.
- CLI one-shot commands plus `actions`, `agent` with typed settings flags
  including available-file-path and sensitive-data placeholder context,
  OpenAI-wire structured-output mode overrides, system-message control,
  `mcp-tools`, `mcp-stdio`, and local persistent `session` commands.
- MCP stdio tools for state, actions, and agent runs, including typed
  `AgentSettings`, OpenAI-wire structured-output mode overrides, in-process
  session reuse by `session_id`, and reconnection to persistent CLI session
  records.
- MCP stdio persistent session lifecycle for start, stop, and list.
- Local TCP newline-delimited JSON-RPC daemon and HTTP JSON-RPC daemon exposing
  the MCP tool surface with shared in-process sessions across active
  connections, `GET /healthz`, and optional bearer/header token auth for
  `POST /rpc`, plus graceful signal shutdown, supervisor pid/ready files, and
  packaged systemd/launchd templates for long-lived local installs.
- Release tarballs include daemon supervision docs plus systemd and launchd
  templates alongside the binary and license files.
- Workspace CI for format, clippy, unit tests, schema fixtures, and conformance
  fixtures.

## Known Gaps

- Cross-origin iframe fallback traversal is limited to Chrome OOPIF target
  sessions; stale-node fallback still searches the current main-frame DOM.
- Browser profile lifecycle support now exposes bounded public lifecycle
  diagnostics for core browser/target/navigation/security transitions and
  stable event shapes for reconnect, target-crash/network-timeout, JavaScript
  dialog, download, and storage-state lifecycle diagnostics. Live CDP wiring now
  records target crash, JavaScript dialog, navigation failure, configured
  download events, cookie plus attached frame-tree origin storage-state
  save/load events, explicit CDP websocket closure diagnostics, bounded
  actor-level reconnect attempts, deliberate stale-session invalidation and
  current-target reattach after reconnect, direct navigation timeouts, and
  watchdog-style stuck HTTP(S) request timeouts. `subscribe_lifecycle_events`
  exposes those diagnostics through `BrowserLifecycleEventSubscription` with
  typed lag and closed-stream errors; `BrowserLifecycleAdapterEventSubscription`
  maps the same stream into upstream-style subscriber categories without adding
  it to normal agent replies. Profile-wide local/session storage discovery
  outside the current page plus attached frame tree remains outside the safe CDP
  boundary documented in `docs/CONFORMANCE.md`.
- Accessibility-tree parity is partial; the DOM serializer now carries common
  AX role/name/state/value properties but still uses a pragmatic compact
  representation rather than full browser-use AX snapshots.
- Browser/action calls that implicitly create MCP sessions are still in-process
  only and are lost when the stdio server exits.
- CLI sessions are local registry records; there is not yet a supervised
  background service that owns their lifecycle.
- The packaged daemon service files are local user-service templates; distro
  packages, Homebrew formulas, and installer-managed secret stores are not
  implemented.
- Provider-specific structured-output fallbacks for gateway routing hints and
  non-chat-completions providers are still partial; DeepSeek now has a forced
  tool-call fallback.
- Managed filesystem and agent checkpoint replay now cover serialized restore
  into a new agent, restored prompt context, restored `read_file`, todo
  context, extracted-content numbering, prior history, and initial-action
  execution state.
- Package publishing is limited to the GitHub release artifact.
