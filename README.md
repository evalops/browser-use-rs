# browser-use-rs

`browser-use-rs` is an EvalOps Rust port of
[`browser-use`](https://github.com/browser-use/browser-use), designed as a
behavioral conformance implementation rather than a line-by-line translation.

The first frozen upstream target is:

```text
browser-use/browser-use@933e28c599ddd74c15a48568f159da95547e40dd
```

## Status

This repository is brand new. The initial milestones are:

1. Workspace, CI, license, and attribution.
2. Typed Rust contracts for browser state, DOM state, actions, action results,
   LLM requests, and agent history.
3. CDP-backed browser session primitives: launch/connect, tabs, navigation,
   screenshots, and page state.
4. DOM and accessibility snapshot serialization compatible with browser-use's
   numbered element/action model.
5. Built-in tools: `navigate`, `search`, `click`, `input`, `scroll`,
   `send_keys`, `screenshot`, `extract`, and `done`.
6. Agent loop: state construction, schema-guided LLM output, retries, step
   limits, page-change guards, loop detection, and history.
7. CLI, daemon sessions, MCP server, and conformance harnesses.

## Design Rules

- Preserve behavior and contracts before optimizing API aesthetics.
- Prefer typed contracts, explicit timeouts, and cancellable async boundaries.
- Treat browser-use Python tests and docs as conformance inputs.
- Keep small commits pushed frequently so every slice is rollbackable.
- Attribute upstream clearly and keep compatibility drift visible.

## Workspace

- `browser-use-core`: agent state, history, settings, and shared result types.
- `browser-use-cdp`: browser launch/connect/session primitives.
- `browser-use-dom`: DOM, accessibility, and selector-map types.
- `browser-use-tools`: built-in action schemas and registry contracts.
- `browser-use-llm`: provider trait and model request/response types.
- `browser-use-cli`: command-line entrypoint and daemon surface.
- `browser-use-mcp`: MCP bridge.
- `browser-use-conformance`: golden fixtures and parity test utilities.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
