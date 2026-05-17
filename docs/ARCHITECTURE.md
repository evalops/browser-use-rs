# Architecture

The Rust port keeps browser-use's user-facing model intact while replacing the
Python internals with smaller typed Rust boundaries.

## Compatibility Target

The initial target is `browser-use/browser-use` commit
`933e28c599ddd74c15a48568f159da95547e40dd`, observed with release `0.12.6` as
the latest published release. Future upstream bumps should land as explicit
conformance updates, not silent rewrites.

## Layers

1. `browser-use-cdp` owns browser process/session lifecycle, Chrome DevTools
   Protocol transport, targets, tabs, downloads, screenshots, and timeouts.
2. `browser-use-dom` turns raw DOM, accessibility, layout, iframe, and snapshot
   data into the compact numbered state sent to an LLM.
3. `browser-use-tools` defines action schemas and dispatches built-in actions
   against a browser session.
4. `browser-use-llm` exposes a provider trait that returns either text or
   schema-validated agent output.
5. `browser-use-core` coordinates the agent loop: prepare state, call the LLM,
   execute one or more actions, detect loops/page changes, record history, and
   stop cleanly.
6. `browser-use-cli` and `browser-use-mcp` expose the same core through human
   and tool-facing front doors.

## Non-Goals

- A literal Python class translation.
- Provider-specific shortcuts in core agent logic.
- Silent divergence from upstream action names or state semantics.
- Unbounded background watchers or hidden long-running poll loops.

## First Useful Slice

The first externally useful release should support:

- `browser-use-rs open <url>`
- `browser-use-rs state --json`
- `browser-use-rs screenshot --output <path>`
- `browser-use-rs click <index>`
- `browser-use-rs type <index> <text>`

That CLI slice forces the browser, DOM, selector map, and action layers to
become real before the LLM agent loop sits on top.
