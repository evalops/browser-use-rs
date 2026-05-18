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
  resistance, and optional IP-address blocking.
- Browser state with URL, title, tabs plus browser-use-style short tab ids,
  screenshots, page metrics, compact DOM state, element bounds, open
  shadow-root indexing, same-origin iframe tag and content indexing, scrollable
  element metadata, automation-friendly data/ARIA/value attributes,
  static history-matching attributes, accessibility-tree role/name enrichment
  with backend/frontend node ids, hidden-element filtering, and basic
  accessible names from labels, ARIA references, and image alt text, plus
  selected dropdown values, common ARIA widget roles, search affordance signals,
  small icon controls, input mask/autocomplete/date-format hints, JavaScript
  click/pointer listener-backed controls, cursor-pointer controls, static
  mouse/keyboard handler attributes, pagination affordance detection,
  configurable prompt-visible attributes, and the upstream empty-DOM load hint.
- Built-in actions for search, navigate, back navigation, 4-character tab-id
  switch/close, click, coordinate click, input, page or indexed element scroll,
  wait, text-target scroll, browser JavaScript evaluation, screenshot, native and
  ARIA dropdown options/selection, keyboard text/special-key/shortcut events,
  file upload, local text-file read/write/replace, PDF/DOCX text extraction,
  PNG/JPEG image-file reads with one-shot image prompt parts, PDF capture,
  extraction, page search, element lookup, cached observed-node
  click/input/scroll/dropdown/upload resolution, and done.
- `screenshot` requests screenshot inclusion in the next observation by default
  and writes a local `.png` file with an attachment path when `file_name` is
  supplied.
- `save_as_pdf` writes a local PDF file, appends `.pdf` when missing, derives a
  safe page-title filename when omitted, avoids overwriting existing files, and
  returns the saved file as an attachment.
- `done.files_to_display` appends readable requested text files to the final
  result and returns their attachment paths.
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
  model-output, model-action, thought, duration, and URL accessors.
- Schema-guided extraction results include structured metadata with schema,
  partial status, content statistics, link/image counts, and de-duplication
  counts.
- Scripted agent replay conformance fixture for schema-guided model output,
  previous-result prompt context, action execution, `done`, and serialized
  history, with semantic checks for dynamic step timing metadata.
- OpenAI-compatible Chat Completions, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output requests.
- CLI one-shot commands plus `actions`, `agent` with typed settings flags
  including available-file-path and sensitive-data placeholder context plus
  system-message control, `mcp-tools`, `mcp-stdio`, and local persistent
  `session` commands.
- MCP stdio tools for state, actions, and agent runs, including typed
  `AgentSettings`, in-process session reuse by `session_id`, and reconnection
  to persistent CLI session records.
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

- Cross-origin iframe interaction is not implemented.
- Browser profile URL access policies currently guard explicit navigation
  requests; post-redirect and unsolicited new-tab watchdog enforcement remains
  lighter than upstream.
- Accessibility-tree parity is partial; the DOM serializer currently uses a
  pragmatic compact representation rather than full browser-use AX snapshots.
- Browser/action calls that implicitly create MCP sessions are still in-process
  only and are lost when the stdio server exits.
- CLI sessions are local registry records; there is not yet a supervised
  background service that owns their lifecycle.
- The packaged daemon service files are local user-service templates; distro
  packages, Homebrew formulas, and installer-managed secret stores are not
  implemented.
- Provider parity beyond OpenAI-compatible Chat Completions, Anthropic Messages,
  Gemini GenerateContent, and Ollama Chat is not implemented.
- Rich filesystem state and sandboxing are still lighter than upstream's
  `FileSystem` service.
- Package publishing is limited to the GitHub release artifact.
