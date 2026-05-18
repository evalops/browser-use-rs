# Roadmap

This repo is being built as a behavioral conformance port of browser-use, with
small pushed checkpoints as each surface becomes real.

## Active Tracks

- [#30 Add active CDP reconnect manager and multi-origin storage discovery](https://github.com/evalops/browser-use-rs/issues/30)

## Completed Tracks

- [#1 Implement CDP-backed local Chrome sessions](https://github.com/evalops/browser-use-rs/issues/1)
- [#2 Build CLI, MCP, and conformance release surface](https://github.com/evalops/browser-use-rs/issues/2)
- [#3 Implement DOM and accessibility serializer parity](https://github.com/evalops/browser-use-rs/issues/3)
- [#4 Implement agent loop and LLM provider contracts](https://github.com/evalops/browser-use-rs/issues/4)
- [#8 Cut first public conformance release](https://github.com/evalops/browser-use-rs/issues/8)
- [#9 Add event-driven browser security watchdog](https://github.com/evalops/browser-use-rs/issues/9)
- [#10 Expand provider-specific structured output fallbacks](https://github.com/evalops/browser-use-rs/issues/10)
- [#12 Port upstream FileSystem CSV normalization](https://github.com/evalops/browser-use-rs/issues/12)
- [#13 Expand browser-profile watchdog diagnostics](https://github.com/evalops/browser-use-rs/issues/13)
- [#14 Match upstream append_file missing-file behavior](https://github.com/evalops/browser-use-rs/issues/14)
- [#15 Support DOCX write_file artifacts](https://github.com/evalops/browser-use-rs/issues/15)
- [#16 Support PDF write_file artifacts](https://github.com/evalops/browser-use-rs/issues/16)
- [#17 Paginate PDF write_file artifacts](https://github.com/evalops/browser-use-rs/issues/17)
- [#18 Add upstream-style PDF read_file page envelopes](https://github.com/evalops/browser-use-rs/issues/18)
- [#19 Sanitize relative file action filenames like upstream FileSystem](https://github.com/evalops/browser-use-rs/issues/19)
- [#20 Align file action binary extension guards](https://github.com/evalops/browser-use-rs/issues/20)
- [#21 Introduce managed FileSystem state and sandbox directory](https://github.com/evalops/browser-use-rs/issues/21)
- [#22 Add managed FileSystem sandbox directory for relative file actions](https://github.com/evalops/browser-use-rs/issues/22)
- [#23 Expose managed FileSystem in agent prompt and state lifecycle](https://github.com/evalops/browser-use-rs/issues/23)
- [#24 Add managed FileSystem replay and restored-agent conformance](https://github.com/evalops/browser-use-rs/issues/24)
- [#25 Expose serializable Agent checkpoint and resume API](https://github.com/evalops/browser-use-rs/issues/25)
- [#27 Expand browser profile lifecycle event hooks](https://github.com/evalops/browser-use-rs/issues/27)
- [#28 Expose remaining browser lifecycle event hooks](https://github.com/evalops/browser-use-rs/issues/28)
- [#29 Add reconnect recovery and deeper lifecycle watchdog automation](https://github.com/evalops/browser-use-rs/issues/29)

## Current Checkpoint

Implemented:

- Public repository, MIT license, upstream attribution, CI, and Rust workspace.
- Published `v0.1.0` public conformance release with tag-triggered GitHub
  release workflow, release support matrix, and packaged Linux artifact smoke.
- Frozen upstream target: `browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd`.
- Core action, browser state, LLM, and history contracts.
- Multi-action execution guard behavior for navigation, `done`, errors, and sequence-terminating actions.
- Browser-backed action executor contract over a CDP session trait.
- Built-in browser action parity for wait/sleep steps with the upstream default
  and bounded delay behavior, browser-history back navigation, and text-target
  scrolling, plus browser JavaScript evaluation.
- Screenshot action behavior for next-observation screenshot requests and
  optional local `.png` file writes with attachment paths.
- PDF save action behavior for local files, including default page-title names,
  `.pdf` extension normalization, duplicate filename avoidance, and attachment
  paths.
- Built-in text-file read/write/replace action parity for local agent artifacts,
  upstream-style CSV write/append normalization, relative filename
  sanitization for local file actions, page-aware PDF read envelopes, PDF/DOCX
  write/append artifacts with paginated PDF text layout, plus upstream-style
  DOCX text extraction and PNG/JPEG read payloads for one-shot image prompt
  parts, with upstream-aligned binary/image extension rejection. Append mode
  requires an existing file, matching upstream `FileSystem` semantics.
- Managed `FileSystem` state with a `browseruse_agent_data` sandbox directory,
  default `todo.md`, file listing/display, extract-content numbering,
  serialization/restoration, nuke, and disk sync for text, CSV, PDF, and DOCX
  artifacts. Executor-owned relative file actions and `done.files_to_display`
  route through that sandbox while absolute external paths bypass it. Agent
  prompts include upstream-style `<file_system>` and `<todo_contents>` context,
  and large extract results can spill into managed `extracted_content_N.md`
  files. Restored agents can continue prompt and tool execution from serialized
  `FileSystemState`, including restored `read_file` behavior, todo/report
  context, and extracted-content numbering that survives replay.
- `done.files_to_display` parity for appending readable text files to the final
  result and returning attachment paths.
- Browser profile launch planning and Chrome `DevToolsActivePort` endpoint parsing.
- Browser profile URL access policies for explicit navigation, including
  allowed/prohibited domain patterns, allowed-domain precedence, internal
  browser URL allowances, data/blob URL allowances, authentication-bypass
  resistance, optional IP-address blocking, post-navigation redirect reset to
  `about:blank`, blocked-navigation preflight diagnostics,
  navigation-capable action-boundary checks, newly observed disallowed tab
  closure, and event-driven target/frame navigation watchdog enforcement with
  bounded success and failure diagnostics while a session is active. CDP
  sessions expose bounded `BrowserLifecycleEvent` diagnostics for browser
  connect/close, target create/switch/close/crash,
  navigation start/complete/failure/timeout, URL-policy block/reset/popup
  outcomes, reconnect, JavaScript dialog, download, and storage-state event
  shapes without placing the full lifecycle stream into normal agent replies.
  Live CDP wiring records target crash, JavaScript dialog, navigation failure,
  configured download events, cookie plus attached frame-tree origin
  storage-state save/load events, CDP websocket closure diagnostics, and direct
  `Page.navigate` plus stuck HTTP(S) request timeout diagnostics. Unexpected
  websocket drops trigger bounded actor-level reconnect attempts with
  reconnecting/reconnected/failure lifecycle diagnostics.
- CDP WebSocket session for navigation, URL/title/tab state with browser-use
  short tab ids, 4-character tab-id switching/closing, screenshots, PDF
  capture, file uploads, coordinate clicks, keyboard text/special-key/shortcut
  events, native/ARIA dropdown actions, scroll, and compact DOM-indexed browser
  actions, including indexed element scrolling and cached observed-node
  click/input/scroll/dropdown/upload resolution, plus page text and element
  lookup across Chrome OOPIF iframe targets.
- One-shot CLI commands and a stdio MCP server backed by the CDP session,
  including in-process MCP session reuse by `session_id`.
- Local persistent CLI session records for start/state/actions/stop/list across
  CLI invocations.
- MCP stdio reconnection to persistent CLI session records by `session_id`.
- MCP stdio persistent session lifecycle tool for start/stop/list.
- Single-step and bounded agent loops with schema-guided model output, history,
  upstream-style initial actions, max-step, max-failure handling,
  upstream-style max-action truncation, step and LLM timeouts, upstream-style
  page-stat prompt context with loading/skeleton hints, vision-aware screenshot
  capture, screenshot action next-observation image overrides, action-result
  image prompt parts, one-time extraction replay handling, invalid model-output
  recovery,
  upstream-style final `done` responses after repeated failures,
  normalized repeated-action loop detection, loop-awareness prompt nudges,
  upstream flattened planning fields, custom
  prompt-visible DOM attributes, configurable planning prompt nudges,
  structured extraction metadata, per-step timing metadata, thinking/flash
  output-schema controls, upstream-style flattened required output fields,
  upstream-style prompt-history inclusion and limits, clickable-element text
  limits, upstream-style one-time read-state prompt blocks, upstream-style tagged
  agent-history/agent-state/browser-state prompt sections, upstream-style
  available-file-path and sensitive-data placeholder context with `bu_2fa_code`
  TOTP generation, system-message override/extension controls, upstream-style
  last-result completion helpers, upstream-style prompt context/error
  truncation, upstream-compatible action-result success validation, judgement
  results, per-step error slots, model-output/action/thought accessors,
  truncated action-history helpers, duration helpers, and screenshot/URL
  helpers. `AgentCheckpoint` export/resume preserves task settings, history,
  initial-action execution state, and managed filesystem state across a new
  model/session.
- OpenAI-compatible Chat Completions plus DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, and Vercel AI Gateway aliases, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output request
  payloads, including DeepSeek forced tool-call and Cerebras prompt-only
  structured-output modes.
- One-shot CLI agent command backed by explicit provider selection and typed
  agent settings flags.
- MCP tool contract schemas and stdio JSON-RPC tool execution for state,
  actions, and provider-selectable agent runs with typed agent settings.
- CLI agent settings expose available-file-path and sensitive-data placeholder
  context plus system-message override/extension.
- Local TCP newline-delimited JSON-RPC daemon and HTTP JSON-RPC daemon exposing
  the same MCP tools as stdio, including health checks, optional HTTP auth,
  graceful signal shutdown, and supervisor pid/ready files.
- Packaged systemd and launchd daemon supervision templates with documented
  paths, environment, lifecycle files, and health-check smokes.
- Conformance fixtures cover scripted agent replay with schema-guided model
  outputs, previous-result context, browser action execution, `done`,
  serialized history, and managed `FileSystemState` replay through restored
  prompts, todo context, restored `read_file`, extracted-content numbering, and
  full `AgentCheckpoint` resume with prior history and initial-action state,
  public browser lifecycle event JSON shape, plus semantic step timing metadata
  checks.
- DOM serializer marks scrollable indexed elements, indexes same-origin iframe
  tags and contents, indexes Chrome OOPIF cross-origin iframe targets with
  cached-node actions, indexes common ARIA widget roles and disclosure elements,
  enriches indexed elements with browser accessibility-tree roles, names,
  state/value properties, and backend/frontend node ids, carries image alt text
  into image-only control names, renders selected dropdown values, compound
  control metadata, and compact select option summaries, preserves
  automation-friendly data/ARIA/value attributes, native boolean/read-only
  state, validation patterns, `data-state`,
  input mask/autocomplete/date-format datepicker hints, live-region and
  hierarchy metadata, and static
  history-matching attributes, renders native and text datepicker input format
  hints, indexes tabindex-backed controls including `tabindex="-1"`, ARIA
  required/autocomplete/keyshortcut interactivity signals, renders keyboard
  shortcuts as `keyshortcuts`, renders numeric ARIA value metadata in AX-shaped
  fields, detects search affordance signals and small icon controls, indexes
  contenteditable editor variants and media controls, detects
  JavaScript click/pointer listener-backed controls and cursor-pointer
  controls, prunes duplicate long attribute values, prunes decorative SVG child
  elements, detects static mouse/keyboard
  handler attributes, prunes contained duplicate descendants inside action
  containers, detects pagination affordances, carries page-shape stats for
  agent prompts, supports caller-selected prompt attributes, renders the
  upstream empty-DOM load hint, filters occluded elements with a topmost-center
  check, keeps hidden file-input upload targets, indexes plain scroll containers
  without interactive descendants, renders pages-above/below scroll context for
  indexed scroll containers, indexes href-less anchor tags and tabindex-backed
  controls including `tabindex="-1"`, renders human-readable value text, prunes
  non-content
  `head`/`script`/`style`/metadata tags, and excludes hidden, disabled, or
  `data-browser-use-exclude` subtrees from the selector map.
- `browser-use-dom` now also exposes a tree-shaped eval/judge representation
  that mirrors upstream `DOMEvalSerializer` markers for backend-node
  interactives, shadow roots, iframe contents, compact key attributes, scroll
  context, collapsed SVG contents, live CDP capture, and Chrome OOPIF child
  target merging.

Next:

1. Continue [#30](https://github.com/evalops/browser-use-rs/issues/30) by
   wiring session rehydration after reconnect and profile-wide storage
   discovery outside the attached frame tree into live CDP/session behavior.
2. Expand agent planning depth and replay coverage for longer multi-step tasks.
