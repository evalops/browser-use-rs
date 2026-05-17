# Roadmap

This repo is being built as a behavioral conformance port of browser-use, with
small pushed checkpoints as each surface becomes real.

## Active Tracks

- [#1 Implement CDP-backed local Chrome sessions](https://github.com/evalops/browser-use-rs/issues/1)
- [#2 Build CLI, MCP, and conformance release surface](https://github.com/evalops/browser-use-rs/issues/2)
- [#3 Implement DOM and accessibility serializer parity](https://github.com/evalops/browser-use-rs/issues/3)
- [#4 Implement agent loop and LLM provider contracts](https://github.com/evalops/browser-use-rs/issues/4)

## Current Checkpoint

Implemented:

- Public repository, MIT license, upstream attribution, CI, and Rust workspace.
- Frozen upstream target: `browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd`.
- Core action, browser state, LLM, and history contracts.
- Multi-action execution guard behavior for navigation, `done`, errors, and sequence-terminating actions.
- Browser-backed action executor contract over a CDP session trait.
- Browser profile launch planning and Chrome `DevToolsActivePort` endpoint parsing.

Next:

1. Implement the local Chrome launcher and CDP connection behind `browser-use-cdp`.
2. Capture real page URL/title/screenshot state from a local browser.
3. Add deterministic local HTML fixtures for DOM and action conformance.
