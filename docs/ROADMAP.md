# Roadmap

This repo is being built as a behavioral conformance port of browser-use, with
small pushed checkpoints as each surface becomes real.

## Active Tracks

- Next upstream parity slice after #100 lands and CI is green.

## Completed Tracks

- [#100 Add upstream task_id agent identity preservation](https://github.com/evalops/browser-use-rs/issues/100)
- [#98 Add upstream add_new_task follow-up control](https://github.com/evalops/browser-use-rs/issues/98)
- [#96 Add upstream pause/resume agent state](https://github.com/evalops/browser-use-rs/issues/96)
- [#97 Add upstream judge_llm routing](https://github.com/evalops/browser-use-rs/issues/97)
- [#94 Add upstream page_extraction_llm routing](https://github.com/evalops/browser-use-rs/issues/94)
- [#93 Add upstream fallback LLM switching](https://github.com/evalops/browser-use-rs/issues/93)
- [#92 Add upstream extraction_schema agent setting](https://github.com/evalops/browser-use-rs/issues/92)
- [#91 Add upstream file_system_path agent setting](https://github.com/evalops/browser-use-rs/issues/91)
- [#90 Add upstream long-URL shortening and output restoration](https://github.com/evalops/browser-use-rs/issues/90)
- [#89 Add upstream llm_screenshot_size resizing and coordinate scaling](https://github.com/evalops/browser-use-rs/issues/89)
- [#88 Add upstream agent callback and stop-control hooks](https://github.com/evalops/browser-use-rs/issues/88)
- [#87 Add upstream sample_images prompt support](https://github.com/evalops/browser-use-rs/issues/87)
- [#86 Add upstream directly_open_url initial navigation](https://github.com/evalops/browser-use-rs/issues/86)
- [#85 Add BrowserProfile proxy bypass launch parity](https://github.com/evalops/browser-use-rs/issues/85)
- [#84 Add upstream step-budget warning prompt](https://github.com/evalops/browser-use-rs/issues/84)
- [#83 Add upstream max_steps final done guard](https://github.com/evalops/browser-use-rs/issues/83)
- [#82 Add upstream-style per-action timeout guard](https://github.com/evalops/browser-use-rs/issues/82)
- [#81 Add Browser Use Cloud error context parity](https://github.com/evalops/browser-use-rs/issues/81)
- [#80 Add Browser Use Cloud extra headers and timeout parity](https://github.com/evalops/browser-use-rs/issues/80)
- [#79 Add Browser Use Cloud auth config fallback](https://github.com/evalops/browser-use-rs/issues/79)
- [#78 Add Browser Use Cloud stop session contract](https://github.com/evalops/browser-use-rs/issues/78)
- [#75 Add LLM-backed extract action results](https://github.com/evalops/browser-use-rs/issues/75)
- [#74 Add Browser Use Cloud session creation contract](https://github.com/evalops/browser-use-rs/issues/74)
- [#73 Add runtime GIF generation for generate_gif](https://github.com/evalops/browser-use-rs/issues/73)
- [#72 Add upstream-style message compaction support](https://github.com/evalops/browser-use-rs/issues/72)
- [#71 Preserve upstream non-judge auxiliary AgentSettings flags](https://github.com/evalops/browser-use-rs/issues/71)
- [#70 Add upstream-style judge trace validation settings](https://github.com/evalops/browser-use-rs/issues/70)
- [#69 Add upstream-style conversation transcript saving](https://github.com/evalops/browser-use-rs/issues/69)
- [#68 Add upstream-style upload_file availability validation](https://github.com/evalops/browser-use-rs/issues/68)
- [#67 Add upstream-style auto vision mode and screenshot action gating](https://github.com/evalops/browser-use-rs/issues/67)
- [#66 Add display_files_in_done_text setting](https://github.com/evalops/browser-use-rs/issues/66)
- [#65 Add upstream-style vision detail level controls](https://github.com/evalops/browser-use-rs/issues/65)
- [#64 Add upstream-style include_recent_events prompt control](https://github.com/evalops/browser-use-rs/issues/64)
- [#63 Reject excluded model actions before execution](https://github.com/evalops/browser-use-rs/issues/63)
- [#62 Add upstream-style excluded action schema controls](https://github.com/evalops/browser-use-rs/issues/62)
- [#61 Harden release browser smoke against Chrome startup flakes](https://github.com/evalops/browser-use-rs/issues/61)
- [#60 Publish generated formula to EvalOps Homebrew tap](https://github.com/evalops/browser-use-rs/issues/60)
- [#59 Add OpenRouter app attribution headers](https://github.com/evalops/browser-use-rs/issues/59)
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
- [#30 Add active CDP reconnect manager and multi-origin storage discovery](https://github.com/evalops/browser-use-rs/issues/30)
- [#31 Add profile-wide storage inventory and full lifecycle event bus parity](https://github.com/evalops/browser-use-rs/issues/31)
- [#32 Add upstream-style lifecycle adapter taxonomy](https://github.com/evalops/browser-use-rs/issues/32)
- [#33 Expand long-task planning replay coverage](https://github.com/evalops/browser-use-rs/issues/33)
- [#34 Improve stale-node fallback across iframe target sessions](https://github.com/evalops/browser-use-rs/issues/34)
- [#35 Expand accessibility snapshot parity](https://github.com/evalops/browser-use-rs/issues/35)
- [#36 Persist implicit MCP session_id launches](https://github.com/evalops/browser-use-rs/issues/36)
- [#37 Report persistent session liveness in registry outputs](https://github.com/evalops/browser-use-rs/issues/37)
- [#38 Capture interacted element metadata in agent history](https://github.com/evalops/browser-use-rs/issues/38)
- [#39 Add daemon-owned session supervision and stale cleanup](https://github.com/evalops/browser-use-rs/issues/39)
- [#40 Implement interacted-element rematching for history replay](https://github.com/evalops/browser-use-rs/issues/40)
- [#41 Apply interacted-element rematches to historical action replay](https://github.com/evalops/browser-use-rs/issues/41)
- [#42 Build rematched replay plans from AgentHistory](https://github.com/evalops/browser-use-rs/issues/42)
- [#43 Execute rematched replay plans against browser sessions](https://github.com/evalops/browser-use-rs/issues/43)
- [#44 Honor browser page-change guards during replay execution](https://github.com/evalops/browser-use-rs/issues/44)
- [#45 Replay AgentHistory against current browser state](https://github.com/evalops/browser-use-rs/issues/45)
- [#46 Freeze AgentHistoryReplayRun conformance fixture](https://github.com/evalops/browser-use-rs/issues/46)
- [#47 Expose AgentHistory replay through the CLI](https://github.com/evalops/browser-use-rs/issues/47)
- [#50 Expose AgentHistory replay through MCP and daemon](https://github.com/evalops/browser-use-rs/issues/50)
- [#51 Expose AgentHistoryReplayRun JSON Schema](https://github.com/evalops/browser-use-rs/issues/51)
- [#52 Replay AgentHistory against persistent CLI sessions](https://github.com/evalops/browser-use-rs/issues/52)
- [#53 Expose MCP output schemas for structuredContent](https://github.com/evalops/browser-use-rs/issues/53)
- [#54 Recapture and rematch DOM state between replay actions](https://github.com/evalops/browser-use-rs/issues/54)
- [#55 Pin replay recapture in conformance fixtures](https://github.com/evalops/browser-use-rs/issues/55)
- [#56 Add package release install surfaces](https://github.com/evalops/browser-use-rs/issues/56)
- [#57 Publish macOS release artifacts](https://github.com/evalops/browser-use-rs/issues/57)
- [#58 Teach Homebrew formula to select macOS release assets](https://github.com/evalops/browser-use-rs/issues/58)

## Current Checkpoint

Implemented:

- Public repository, MIT license, upstream attribution, CI, and Rust workspace.
- Published `v0.1.0` public conformance release with tag-triggered GitHub
  release workflow, release support matrix, packaged Linux artifact smoke,
  macOS host-triple artifact smoke, cross-tarball SHA-256 checksum metadata,
  install guide, and generated platform-aware Homebrew formula scaffold.
- Frozen upstream target: `browser-use/browser-use@f09a86671591312bbc272403a7409d56f4cec668`.
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
  sanitization for local file actions, upstream-style agent `upload_file`
  availability checks, page-aware PDF read envelopes, PDF/DOCX write/append
  artifacts with paginated PDF text layout, plus upstream-style DOCX text
  extraction and PNG/JPEG read payloads for one-shot image prompt parts, with
  upstream-aligned binary/image extension rejection. Append mode requires an
  existing file, matching upstream `FileSystem` semantics.
- Managed `FileSystem` state with a `browseruse_agent_data` sandbox directory,
  default `todo.md`, file listing/display, extract-content numbering,
  serialization/restoration, nuke, and disk sync for text, CSV, PDF, and DOCX
  artifacts. Executor-owned relative file actions and `done.files_to_display`
  route through that sandbox while absolute external paths bypass it. Agent
  prompts include upstream-style `<file_system>` and `<todo_contents>` context,
  and large extract results can spill into managed `extracted_content_N.md`
  files. Agent-level `extraction_schema` supplies the default structured schema
  for LLM-backed extract actions that do not provide their own `output_schema`.
  Restored agents can continue prompt and tool execution from serialized
  `FileSystemState`, including restored `read_file` behavior, todo/report
  context, and extracted-content numbering that survives replay. Agents can use
  upstream-style `file_system_path` to place the managed filesystem under a
  caller-selected base directory while preserving the `browseruse_agent_data`
  subdirectory contract.
- `done.files_to_display` parity for appending readable text files to the final
  result and returning attachment paths.
- Browser profile launch planning, including upstream-style proxy-server and
  proxy-bypass Chrome flags, and Chrome `DevToolsActivePort` endpoint parsing.
- Browser Use Cloud creation and stop request/response contracts, including
  `BROWSER_USE_API_KEY`/explicit-key client support, `cloud_auth.json`
  API-token fallback, 30-second request timeout, extra request headers merged
  after default auth/content-type headers, current-session tracking after
  create, explicit or current-session stop requests, auth errors,
  missing-session errors, action-specific create/stop Cloud error context,
  current-session cleanup on successful stop or 404, `cdpUrl` to CDP endpoint
  conversion, and upstream-compatible omitted/null/country proxy-country
  serialization.
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
  reconnecting/reconnected/failure lifecycle diagnostics, and registered CDP
  target sessions are invalidated after reconnect so stale session-scoped
  commands fail locally with a clear reattach error. The current target is
  reattached automatically on the next session access when Chrome still exposes
  it. `subscribe_lifecycle_events` exposes the lifecycle stream through
  `BrowserLifecycleEventSubscription` without adding it to normal agent replies
  or forcing integrations to handle Tokio broadcast receiver details directly.
  `BrowserLifecycleAdapterEventSubscription` maps the same diagnostics into
  upstream-style subscriber categories for tab, focus, navigation, browser
  error, download, storage-state, dialog, reconnect, and diagnostic events.
- CDP WebSocket session for navigation, URL/title/tab state with browser-use
  short tab ids, 4-character tab-id switching/closing, screenshots, PDF
  capture, file uploads, coordinate clicks, keyboard text/special-key/shortcut
  events, native/ARIA dropdown actions, scroll, and compact DOM-indexed browser
  actions, including indexed element scrolling, cached observed-node
  click/input/scroll/dropdown/upload resolution, target-aware stale-node
  fallback for cached iframe actions, plus page text and element lookup across
  Chrome OOPIF iframe targets.
- One-shot CLI commands, including `AgentHistory` replay against current browser
  state, and a stdio MCP server backed by the CDP session, including
  `browser_use_replay` and in-process MCP session reuse by `session_id`.
- Local persistent CLI session records for start, state, actions, replay, stop,
  list, and cleanup across CLI invocations.
- MCP stdio reconnection to persistent CLI session records by `session_id`, and
  persistent record creation for new `session_id` state/action/agent calls with
  a supplied URL.
- MCP stdio persistent session lifecycle tool for start/stop/list/cleanup, with
  liveness status and conservative stale-record cleanup on session records.
- Single-step and bounded agent loops with schema-guided model output, history,
  upstream-style initial actions, `directly_open_url` task URL extraction and
  step-zero navigation, sync and async new-step/done callbacks,
  callback-driven stop checks, explicit programmatic stop with reasoned stop
  errors, upstream-style pause/resume control state with checkpoint
  preservation, upstream-style continuous follow-up task updates via
  `add_new_task`, max-step, max-failure handling,
  upstream-style max-action truncation, step and LLM timeouts, upstream-style
  per-action wall-clock timeout guard with `BROWSER_USE_ACTION_TIMEOUT_S` and
  `action_timeout_seconds`, validated `llm_screenshot_size` prompt-only PNG
  resizing with coordinate-click scaling back to the observed viewport,
  upstream-style long URL shortening for user/assistant prompt text with
  recursive restoration of parsed model output before execution/history,
  upstream-style fallback LLM switching for retryable main model-output
  provider/rate-limit failures,
  upstream-style final-step done-only guard for
  `max_steps`, upstream-style 75% step-budget warning before finalization,
  upstream-style page-stat prompt context with loading/skeleton hints,
  upstream-compatible
  `true`/`false`/`auto` vision modes, auto-only screenshot action gating,
  upstream-style `sample_images` prompt parts before screenshots, screenshot
  action next-observation image overrides, action-result image prompt parts,
  one-time extraction replay handling, invalid model-output
  recovery,
  upstream-style final `done` responses after repeated failures,
  normalized repeated-action loop detection, loop-awareness prompt nudges,
  upstream flattened planning fields, custom
  prompt-visible DOM attributes, upstream-style excluded-action schema
  controls and pre-execution enforcement, opt-in recent browser events,
  upstream-style vision detail levels, upstream-style `done` file-display
  controls, configurable planning prompt nudges,
  LLM-backed extract action results for free-text and structured-schema
  extraction while preserving raw extraction envelopes for direct/replay
  callers, upstream-style dedicated page-extraction LLM routing, per-step
  timing metadata, thinking/flash output-schema controls,
  upstream-style flattened required output fields,
  upstream-style prompt-history inclusion and limits, clickable-element text
  limits, upstream-style one-time read-state prompt blocks, upstream-style tagged
  agent-history/agent-state/browser-state prompt sections, upstream-style
  available-file-path and sensitive-data placeholder context with `bu_2fa_code`
  TOTP generation, system-message override/extension controls, upstream-style
  last-result completion helpers, upstream-style prompt context/error
  truncation, upstream-compatible action-result success validation,
  upstream-style judge trace validation, judgement results, and dedicated
  judge LLM routing, per-step error slots, runtime `generate_gif` GIF artifact
  output from recorded screenshots,
  contract-preserved `calculate_cost` and `include_tool_call_examples` settings,
  upstream-style message compaction
  settings with non-fatal summary requests, compacted-memory prompt blocks, and
  checkpoint preservation, model-output/action/thought accessors,
  model-action and truncated action-history interacted-element metadata for
  indexed actions, explicit replay action rematching for historical indexed
  actions, rematched replay-plan construction from saved `AgentHistory`,
  replay-plan execution through generic and browser-backed action executors
  with per-action, error, and page-change diagnostics, and current-state
  `AgentHistoryReplayRun` orchestration that recaptures DOM state between
  browser-backed replay actions, rematches later indexed actions against the
  latest DOM, and returns captured state, plan, and execution diagnostics
  covered by a serialized conformance fixture, plus duration and screenshot/URL
  helpers.
  `AgentCheckpoint` export/resume preserves task settings, history,
  initial-action execution state, and managed filesystem state across a new
  model/session.
- OpenAI-compatible Chat Completions plus DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, and Vercel AI Gateway aliases, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output request
  payloads, including DeepSeek forced tool-call, Cerebras prompt-only
  structured-output modes, and OpenRouter app attribution headers.
- One-shot CLI agent command backed by explicit provider selection and typed
  agent settings flags.
- MCP tool input/output contract schemas and stdio JSON-RPC tool execution for
  state, actions, `AgentHistory` replay, and provider-selectable agent runs
  with typed agent settings.
- CLI agent settings expose available-file-path and sensitive-data placeholder
  context plus system-message override/extension.
- Local TCP newline-delimited JSON-RPC daemon and HTTP JSON-RPC daemon exposing
  the same MCP tools as stdio, including health checks, optional HTTP auth,
  graceful signal shutdown, and supervisor pid/ready files.
- Packaged systemd and launchd daemon supervision templates with documented
  paths, environment, lifecycle files, and health-check smokes.
- Conformance fixtures cover scripted agent replay with schema-guided model
  outputs, previous-result context, browser action execution, `done`,
  serialized history, longer multi-step planning/recovery replay with
  prompt-history limits and stagnant-page loop-awareness, and managed
  `FileSystemState` replay through restored prompts, todo context, restored
  `read_file`, extracted-content numbering, and full `AgentCheckpoint` resume
  with prior history and initial-action state, browser-backed replay
  recapture/rematch, public browser lifecycle event and adapter JSON shapes,
  public `AgentHistoryReplayRun` JSON Schema, plus semantic step timing
  metadata checks.
- DOM serializer marks scrollable indexed elements, indexes same-origin iframe
  tags and contents, indexes Chrome OOPIF cross-origin iframe targets with
  cached-node actions, indexes common ARIA widget roles and disclosure elements,
  enriches indexed elements with browser accessibility-tree roles, names,
  descriptions, state/value properties, compact `ax_name`/`ax_description`
  metadata, AX hidden/disabled suppression, quiet AX
  focusable/editable/settable metadata, and backend/frontend node ids, carries
  image alt text into image-only control names, renders selected dropdown
  values, compound control metadata, and compact select option summaries,
  preserves
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
  context, collapsed SVG contents, live CDP capture, Chrome OOPIF child target
  merging, and interacted-element rematching diagnostics for exact, stable,
  XPath, AX-name, and unique-attribute history replay foundations. Core replay
  helpers can remap saved history and execute the resulting action plan through
  generic or browser-backed action-executor boundaries while preserving
  step/action diagnostics and the live executor's URL-change guard. Browser
  executors can also capture the current DOM, recapture state between replay
  actions, rematch later indexed actions against the latest DOM, and return a
  replay run containing the captured state, plan, and guarded execution result,
  with the public JSON shape pinned by `agent_history_replay_run.json` and
  `agent_history_replay_recapture_run.json`.

Next:

1. Open the next narrowly scoped parity issue before starting implementation.
