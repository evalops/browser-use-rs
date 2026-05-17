# Roadmap

This repo is being built as a behavioral conformance port of browser-use, with
small pushed checkpoints as each surface becomes real.

## Active Tracks

- [#2 Build CLI, MCP, and conformance release surface](https://github.com/evalops/browser-use-rs/issues/2)
- [#3 Implement DOM and accessibility serializer parity](https://github.com/evalops/browser-use-rs/issues/3)
- [#4 Implement agent loop and LLM provider contracts](https://github.com/evalops/browser-use-rs/issues/4)

## Completed Tracks

- [#1 Implement CDP-backed local Chrome sessions](https://github.com/evalops/browser-use-rs/issues/1)

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
- Browser profile launch planning and Chrome `DevToolsActivePort` endpoint parsing.
- CDP WebSocket session for navigation, tab switching/closing, URL/title state,
  screenshots, PDF capture, file uploads, coordinate clicks, scroll, and compact
  DOM-indexed browser actions, including indexed element scrolling.
- One-shot CLI commands and a stdio MCP server backed by the CDP session,
  including in-process MCP session reuse by `session_id`.
- Local persistent CLI session records for start/state/actions/stop/list across
  CLI invocations.
- MCP stdio reconnection to persistent CLI session records by `session_id`.
- MCP stdio persistent session lifecycle tool for start/stop/list.
- Single-step and bounded agent loops with schema-guided model output, history,
  max-step, max-failure handling, step and LLM timeouts, compact page-stat
  prompt context, vision-aware screenshot capture and image prompt parts,
  one-time extraction replay handling, invalid model-output recovery,
  final-result, success, and error helpers.
- OpenAI-compatible Chat Completions, Anthropic Messages, Gemini
  GenerateContent, and Ollama Chat providers with structured-output request
  payloads.
- One-shot CLI agent command backed by explicit provider selection.
- MCP tool contract schemas and stdio JSON-RPC tool execution for state,
  actions, and provider-selectable agent runs.
- Local TCP JSON-RPC daemon exposing the same MCP tools as stdio.
- Conformance fixtures cover a scripted agent replay with schema-guided model
  outputs, previous-result context, browser action execution, `done`, and
  serialized history.
- DOM serializer marks scrollable indexed elements, indexes same-origin iframe
  tags and contents, indexes common ARIA widget roles and disclosure elements,
  carries image alt text into image-only control names, and excludes hidden or
  disabled interactive elements from the selector map.

Next:

1. Move compact DOM serialization toward accessibility-aware parity.
2. Harden the network daemon with HTTP, auth, and supervision options.
3. Expand agent planning depth, replay coverage, and provider parity beyond
   OpenAI-compatible, Anthropic, Gemini, and Ollama.
