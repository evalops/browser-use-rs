# Release Support Matrix

This release targets:

```text
browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd
```

## Supported

- Local Chrome/Chromium launch and CDP attach.
- Browser state with URL, title, tabs, screenshots, page metrics, compact DOM
  state, element bounds, open shadow-root indexing, same-origin iframe indexing,
  and basic accessible names from labels and ARIA references.
- Built-in actions for search, navigate, tab switch/close, click, coordinate
  click, input, scroll, screenshot, dropdown options/selection, send keys, file
  upload, PDF capture, extraction, page search, element lookup, and done.
- Browser-aware action sequencing that stops on errors, done, explicit
  terminating actions, and URL changes after browser actions.
- Agent runs with schema-guided provider output, max actions per step, max
  steps, max failures, LLM timeout, repeated-action loop detection, previous
  result context, and typed history/final result.
- OpenAI-compatible Chat Completions provider with structured-output requests.
- CLI one-shot commands plus `actions`, `agent`, `mcp-tools`, and `mcp-stdio`.
- MCP stdio tools for state, actions, and agent runs, including in-process
  session reuse by `session_id`.
- Workspace CI for format, clippy, unit tests, schema fixtures, and conformance
  fixtures.

## Known Gaps

- Cross-origin iframe interaction is not implemented.
- Accessibility-tree parity is partial; the DOM serializer currently uses a
  pragmatic compact representation rather than full browser-use AX snapshots.
- MCP sessions are in-process only and are lost when the stdio server exits.
- CLI daemon sessions are not implemented yet.
- Provider parity beyond OpenAI-compatible Chat Completions is not implemented.
- Package publishing is limited to the GitHub release artifact.
