# MCP

`browser-use-mcp` owns the stable JSON tool contracts used by the stdio MCP
server exposed through:

```sh
browser-use-rs mcp-stdio
browser-use-rs daemon --addr 127.0.0.1:8765
```

The server implements newline-delimited JSON-RPC over stdin/stdout for the MCP
`2025-06-18` lifecycle and tools surface. The daemon exposes the same
newline-delimited JSON-RPC messages over TCP and shares one in-process runtime
across active connections.

Current tool contracts:

- `browser_use_state`: launch a browser, navigate to a URL, and return browser
  state.
- `browser_use_actions`: launch a browser, execute a JSON array of
  `BrowserAction` values, and return action results plus final state.
- `browser_use_agent`: launch a browser, run a bounded agent task, and return
  typed agent history.
- `browser_use_session`: start, stop, and list persistent browser sessions.

Provider secrets are intentionally not part of MCP tool input schemas. A server
implementation reads provider credentials from its process environment or host
configuration. `browser_use_agent` accepts an optional `provider` input:
`openai-compatible` (default) or `anthropic`. OpenAI-compatible runs require
`OPENAI_API_KEY` plus a tool `model` argument or `OPENAI_MODEL`;
`OPENAI_BASE_URL` is optional. Anthropic runs require `ANTHROPIC_API_KEY` plus a
tool `model` argument or `ANTHROPIC_MODEL`; `ANTHROPIC_BASE_URL`,
`ANTHROPIC_VERSION`, and `ANTHROPIC_MAX_TOKENS` are optional.

Browser and agent tools support an optional `session_id` argument. When omitted,
the tool call uses a fresh one-shot browser. When present, the stdio server
reuses an in-process Chrome session for subsequent calls with the same
`session_id`. Use `browser_use_session` with `operation` set to `start` to
create a persistent session record that survives stdio server restarts, `list`
to inspect records, and `stop` to close and remove one.
Persistent sessions created by the CLI are the same record format and can be
stopped through MCP, and MCP-created persistent sessions can be stopped with
`browser-use-rs session stop <id>`.

To inspect the manifest from the CLI:

```sh
browser-use-rs mcp-tools
```

Minimal protocol smoke:

```sh
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | browser-use-rs mcp-stdio
```
