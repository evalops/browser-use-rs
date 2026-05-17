# CLI

The CLI currently provides one-shot browser commands. Each command launches a
fresh local Chrome/Chromium instance, connects through CDP, performs the action,
prints or writes the result, and exits.

## Commands

```sh
browser-use-rs version-target
browser-use-rs schema action
browser-use-rs schema browser-state
browser-use-rs mcp-tools
browser-use-rs mcp-stdio
browser-use-rs open <url>
browser-use-rs state <url> [--screenshot]
browser-use-rs screenshot <url> <output.png>
browser-use-rs click <url> <index>
browser-use-rs type <url> <index> <text>
browser-use-rs scroll <url> [--pages 1.0] [--down]
browser-use-rs actions <url> <actions.json> [--screenshot]
browser-use-rs agent <url> <task> --provider openai-compatible \
  [--api-key <key>] [--model <model>] [--base-url https://api.openai.com/v1] \
  [--max-steps 10]
browser-use-rs agent <url> <task> --provider anthropic \
  [--api-key <key>] [--model <model>] [--base-url https://api.anthropic.com/v1] \
  [--max-steps 10]
browser-use-rs session start <id> <url> [--screenshot]
browser-use-rs session state <id> [--screenshot]
browser-use-rs session actions <id> <actions.json> [--screenshot]
browser-use-rs session stop <id>
browser-use-rs session list
```

## Local Smokes

```sh
cargo run -q -p browser-use-cli -- state \
  "data:text/html,<html><head><title>cli smoke</title></head><body><button>Run</button><input placeholder='Name'></body></html>"

cargo run -q -p browser-use-cli -- type \
  "data:text/html,<html><head><title>cli type</title></head><body><input placeholder='Name'></body></html>" \
  1 EvalOps

cargo run -q -p browser-use-cli -- screenshot \
  "data:text/html,<html><head><title>shot</title></head><body><h1>Screenshot</h1></body></html>" \
  /tmp/browser-use-rs-cli-smoke.png
```

`actions` accepts a JSON array of `BrowserAction` objects, runs them in a
single launched browser session, and prints:

```json
{
  "results": [],
  "state": {}
}
```

Agent runs use the same one-shot browser lifecycle and print typed
`AgentHistory` JSON after the bounded run completes. The default provider is
`openai-compatible` and reads `OPENAI_API_KEY`, `OPENAI_MODEL`, and optional
`OPENAI_BASE_URL` from the environment. `--provider anthropic` reads
`ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL`, optional `ANTHROPIC_BASE_URL`,
`ANTHROPIC_VERSION`, and `ANTHROPIC_MAX_TOKENS`. CLI `--api-key`, `--model`,
and `--base-url` override the provider-specific environment values.

`session` commands persist a local Chrome session across CLI invocations. The
session registry defaults to `~/.browser-use-rs/sessions` and can be overridden
with `BROWSER_USE_RS_STATE_DIR`. Session IDs may contain ASCII letters, digits,
`-`, and `_`.

`mcp-stdio` runs a newline-delimited JSON-RPC MCP server over stdin/stdout. It
supports `initialize`, `ping`, `tools/list`, and `tools/call` for
`browser_use_state`, `browser_use_actions`, and `browser_use_agent`. MCP tool
inputs accept an optional `session_id`; calls with the same `session_id` reuse
the same in-process Chrome session.

## Current Limits

- CLI sessions are local process/session records; they are not shared with the
  MCP stdio in-process session registry.
- DOM indexing is compact and useful, but not yet browser-use DOM/AX parity.
- Indexed actions currently target same-document interactive elements; iframe
  and shadow-root support belong to the DOM parity track.
- Agent runs currently support OpenAI-compatible Chat Completions and Anthropic
  Messages structured-output adapters.
- MCP tools are real over stdio and can reuse in-process sessions by
  `session_id`; they do not yet persist sessions across server restarts.
