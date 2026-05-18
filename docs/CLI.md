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
browser-use-rs daemon [--addr 127.0.0.1:8765] [--transport tcp|http] \
  [--auth-token <token>]
browser-use-rs open <url>
browser-use-rs state <url> [--screenshot]
browser-use-rs screenshot <url> <output.png>
browser-use-rs click <url> <index>
browser-use-rs type <url> <index> <text>
browser-use-rs scroll <url> [--pages 1.0] [--down]
browser-use-rs actions <url> <actions.json> [--screenshot]
browser-use-rs agent <url> <task> --provider openai-compatible \
  [--api-key <key>] [--model <model>] [--base-url https://api.openai.com/v1] \
  [--max-steps 10] [--no-vision] [--max-actions-per-step 5] \
  [--flash-mode] [--include-attribute data-testid]
browser-use-rs agent <url> <task> --provider anthropic \
  [--api-key <key>] [--model <model>] [--base-url https://api.anthropic.com/v1] \
  [--max-steps 10]
browser-use-rs agent <url> <task> --provider gemini \
  [--api-key <key>] [--model <model>] \
  [--base-url https://generativelanguage.googleapis.com/v1beta] [--max-steps 10]
browser-use-rs agent <url> <task> --provider ollama \
  [--model <model>] [--base-url http://localhost:11434] [--max-steps 10]
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
`ANTHROPIC_VERSION`, and `ANTHROPIC_MAX_TOKENS`. `--provider gemini` reads
`GEMINI_API_KEY`, `GEMINI_MODEL`, and optional `GEMINI_BASE_URL`.
`--provider ollama` reads `OLLAMA_MODEL` and optional `OLLAMA_BASE_URL` or
`OLLAMA_HOST`; it does not require an API key. CLI `--api-key`, `--model`, and
`--base-url` override the provider-specific environment values where they
apply.

Agent runs also expose the typed `AgentSettings` knobs used by the MCP agent
tool: `--no-vision`, `--max-failures`, `--max-actions-per-step`,
`--llm-timeout-seconds`, `--step-timeout-seconds`, `--no-loop-detection`,
`--loop-detection-window`, `--no-thinking`, `--flash-mode`, `--no-planning`,
`--planning-replan-on-stall`, `--planning-exploration-limit`,
`--max-history-items`, `--max-clickable-elements-length`, and repeated
`--include-attribute <name>` for prompt-visible DOM attributes.

`session` commands persist a local Chrome session across CLI invocations. The
session registry defaults to `~/.browser-use-rs/sessions` and can be overridden
with `BROWSER_USE_RS_STATE_DIR`. Session IDs may contain ASCII letters, digits,
`-`, and `_`.

`mcp-stdio` runs a newline-delimited JSON-RPC MCP server over stdin/stdout. It
supports `initialize`, `ping`, `tools/list`, and `tools/call` for
`browser_use_state`, `browser_use_actions`, `browser_use_agent`, and
`browser_use_session`. MCP browser and agent tool inputs accept an optional
`session_id`; calls with the same `session_id` reuse the same in-process Chrome
session. `browser_use_session` can start, stop, and list persistent session
records. If `session_id` matches a persistent session record, `mcp-stdio`
reconnects to that Chrome session even after the stdio server process restarts.

`daemon` binds a local listener, prints the bound address on startup, shares one
in-process session runtime across active connections, and uses the same
persistent session registry as the CLI and MCP session tool. The default
`--transport tcp` exposes the same newline-delimited JSON-RPC surface as
`mcp-stdio` to each connection. `--transport http` exposes `GET /healthz` and
`POST /rpc`; the `/rpc` body is the same JSON-RPC request used by stdio/TCP and
returns the JSON-RPC response as JSON. `--auth-token <token>` or
`BROWSER_USE_RS_DAEMON_TOKEN=<token>` requires HTTP clients to send either
`Authorization: Bearer <token>` or `X-Browser-Use-Rs-Token: <token>`.

## Current Limits

- MCP can reconnect to persistent sessions created by `browser-use-rs session
  start` or the `browser_use_session` tool; browser/action calls that create a
  session implicitly are still in-process only.
- DOM indexing is compact and accessibility-aware, including same-origin iframe
  traversal, open shadow-root traversal, AX role/name enrichment,
  backend/frontend node ids, and cached observed-node resolution for
  click/type/scroll/dropdown/upload actions. Full browser-use DOM/AX snapshot
  parity is still tracked separately.
- Agent runs currently support OpenAI-compatible Chat Completions, Anthropic
  Messages, Gemini GenerateContent, and Ollama Chat structured-output adapters.
- MCP tools are real over stdio and can reuse in-process sessions by
  `session_id`; persistent sessions must be created explicitly with the CLI
  session command or `browser_use_session`.
- The daemon is a local TCP or HTTP JSON-RPC surface with optional HTTP
  authentication; production process supervision is still out of scope.
