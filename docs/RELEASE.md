# Release Support Matrix

This release targets:

```text
browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd
```

## Supported

- Local Chrome/Chromium launch and CDP attach.
- Browser state with URL, title, tabs, screenshots, page metrics, compact DOM
  state, element bounds, open shadow-root indexing, same-origin iframe tag and
  content indexing, scrollable element metadata, hidden-element filtering, and
  basic accessible names from labels and ARIA references, plus common ARIA
  widget roles.
- Built-in actions for search, navigate, back navigation, tab switch/close,
  click, coordinate click, input, page or indexed element scroll, wait,
  text-target scroll, browser JavaScript evaluation, screenshot, dropdown
  options/selection, send keys, file upload, PDF capture, extraction, page
  search, element lookup, and done.
- Browser-aware action sequencing that stops on errors, done, explicit
  terminating actions, and URL changes after browser actions.
- Agent runs with schema-guided provider output, max actions per step, max
  steps, max failures, step and LLM timeouts, repeated-action loop detection,
  previous result context, vision-aware screenshot capture and image prompt
  parts, compact page-stat prompt context, one-time extraction replay handling,
  invalid model-output recovery, and typed history/final-result/success/error
  accessors.
- Scripted agent replay conformance fixture for schema-guided model output,
  previous-result prompt context, action execution, `done`, and serialized
  history.
- OpenAI-compatible Chat Completions, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output requests.
- CLI one-shot commands plus `actions`, `agent`, `mcp-tools`, `mcp-stdio`, and
  local persistent `session` commands.
- MCP stdio tools for state, actions, and agent runs, including in-process
  session reuse by `session_id` and reconnection to persistent CLI session
  records.
- MCP stdio persistent session lifecycle for start, stop, and list.
- Local TCP JSON-RPC daemon exposing the MCP tool surface with shared
  in-process sessions across active connections.
- Workspace CI for format, clippy, unit tests, schema fixtures, and conformance
  fixtures.

## Known Gaps

- Cross-origin iframe interaction is not implemented.
- Accessibility-tree parity is partial; the DOM serializer currently uses a
  pragmatic compact representation rather than full browser-use AX snapshots.
- Browser/action calls that implicitly create MCP sessions are still in-process
  only and are lost when the stdio server exits.
- CLI sessions are local registry records; there is not yet a supervised
  background service that owns their lifecycle.
- The daemon is local TCP JSON-RPC only; HTTP, auth, and production supervision
  are not implemented.
- Provider parity beyond OpenAI-compatible Chat Completions, Anthropic Messages,
  Gemini GenerateContent, and Ollama Chat is not implemented.
- Package publishing is limited to the GitHub release artifact.
