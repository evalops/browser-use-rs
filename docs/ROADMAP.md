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
- Frozen upstream target: `browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd`.
- Core action, browser state, LLM, and history contracts.
- Multi-action execution guard behavior for navigation, `done`, errors, and sequence-terminating actions.
- Browser-backed action executor contract over a CDP session trait.
- Browser profile launch planning and Chrome `DevToolsActivePort` endpoint parsing.
- Minimal CDP WebSocket session for navigation, URL/title state, screenshots,
  coordinate clicks, scroll, and compact DOM-indexed click/input actions.
- One-shot CLI commands backed by the CDP session.

Next:

1. Add deterministic local HTML fixtures for DOM and action conformance.
2. Move compact DOM serialization toward accessibility-aware parity.
3. Add persistent CLI/MCP sessions.
