# Roadmap

This repo is being built as a behavioral conformance port of browser-use, with
small pushed checkpoints as each surface becomes real.

## Active Tracks

- [#3 Implement DOM and accessibility serializer parity](https://github.com/evalops/browser-use-rs/issues/3)
- [#4 Implement agent loop and LLM provider contracts](https://github.com/evalops/browser-use-rs/issues/4)

## Completed Tracks

- [#1 Implement CDP-backed local Chrome sessions](https://github.com/evalops/browser-use-rs/issues/1)
- [#2 Build CLI, MCP, and conformance release surface](https://github.com/evalops/browser-use-rs/issues/2)

## Current Checkpoint

Implemented:

- Public repository, MIT license, upstream attribution, CI, and Rust workspace.
- Tag-triggered GitHub release workflow and release support matrix.
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
  plus upstream-style PDF/DOCX text extraction and PNG/JPEG read payloads for
  one-shot image prompt parts.
- `done.files_to_display` parity for appending readable text files to the final
  result and returning attachment paths.
- Browser profile launch planning and Chrome `DevToolsActivePort` endpoint parsing.
- Browser profile URL access policies for explicit navigation, including
  allowed/prohibited domain patterns, allowed-domain precedence, internal
  browser URL allowances, data/blob URL allowances, authentication-bypass
  resistance, optional IP-address blocking, post-navigation redirect reset to
  `about:blank`, navigation-capable action-boundary checks, and newly observed
  disallowed tab closure.
- CDP WebSocket session for navigation, URL/title/tab state with browser-use
  short tab ids, 4-character tab-id switching/closing, screenshots, PDF
  capture, file uploads, coordinate clicks, keyboard text/special-key/shortcut
  events, native/ARIA dropdown actions, scroll, and compact DOM-indexed browser
  actions, including indexed element scrolling and cached observed-node
  click/input/scroll/dropdown/upload resolution.
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
  helpers.
- OpenAI-compatible Chat Completions plus DeepSeek, Groq, Cerebras, Mistral,
  OpenRouter, and Vercel AI Gateway aliases, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output request
  payloads, including DeepSeek JSON-object and Cerebras prompt-only
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
- Conformance fixtures cover a scripted agent replay with schema-guided model
  outputs, previous-result context, browser action execution, `done`, and
  serialized history, plus semantic step timing metadata checks.
- DOM serializer marks scrollable indexed elements, indexes same-origin iframe
  tags and contents, indexes common ARIA widget roles and disclosure elements,
  enriches indexed elements with browser accessibility-tree roles, names,
  state/value properties, and backend/frontend node ids, carries image alt text
  into image-only control names, renders selected dropdown values and compact
  select option summaries, preserves automation-friendly data/ARIA/value
  attributes, validation patterns, `data-state`,
  input mask/autocomplete/date-format hints, and static
  history-matching attributes, renders native and text datepicker input format
  hints, detects search affordance signals and small icon controls, detects
  JavaScript click/pointer listener-backed controls and cursor-pointer
  controls, prunes decorative SVG child elements, detects static mouse/keyboard
  handler attributes, prunes contained duplicate descendants inside action
  containers, detects pagination affordances, carries page-shape stats for
  agent prompts, supports caller-selected prompt attributes, renders the
  upstream empty-DOM load hint, filters occluded elements with a topmost-center
  check, keeps hidden file-input upload targets, indexes plain scroll containers
  without interactive descendants, prunes non-content
  `head`/`script`/`style`/metadata tags, and excludes hidden, disabled, or
  `data-browser-use-exclude` subtrees from the selector map.

Next:

1. Move compact DOM serialization toward accessibility-aware parity.
2. Continue browser-profile security parity toward an event-driven watchdog
   when the CDP layer grows background event dispatch.
3. Expand agent planning depth, replay coverage, and deeper provider-specific
   structured-output fallbacks for model families that need tool-calling or
   provider-routing hints.
