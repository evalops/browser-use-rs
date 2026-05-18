# MCP

`browser-use-mcp` owns the stable JSON tool contracts used by the stdio MCP
server exposed through:

```sh
browser-use-rs mcp-stdio
browser-use-rs daemon --addr 127.0.0.1:8765
browser-use-rs daemon --transport http --auth-token <token>
browser-use-rs daemon --pid-file /run/browser-use-rs.pid \
  --ready-file /run/browser-use-rs.ready.json
```

The server implements newline-delimited JSON-RPC over stdin/stdout for the MCP
`2025-06-18` lifecycle and tools surface. The daemon shares one in-process
runtime across active connections. Its default TCP transport exposes the same
newline-delimited JSON-RPC messages over each connection. Its HTTP transport
exposes unauthenticated `GET /healthz` and JSON `POST /rpc`; when
`--auth-token` or `BROWSER_USE_RS_DAEMON_TOKEN` is configured, `/rpc` requires
either `Authorization: Bearer <token>` or
`X-Browser-Use-Rs-Token: <token>`. For long-lived local installs, the daemon
can write supervisor-friendly `--pid-file` and `--ready-file` artifacts after a
successful bind and remove them on graceful Ctrl-C/SIGINT/SIGTERM shutdown.
Packaged systemd and launchd templates are documented in
[DAEMON_SUPERVISION.md](DAEMON_SUPERVISION.md).

Current tool contracts:

- `browser_use_state`: launch a browser, navigate to a URL, and return browser
  state.
- `browser_use_actions`: launch a browser, execute a JSON array of
  `BrowserAction` values, and return action results plus final state.
- `browser_use_replay`: replay saved `AgentHistory` JSON against current
  browser state and return `AgentHistoryReplayRun` with captured state,
  rematched plan, and guarded execution diagnostics.
- `browser_use_agent`: launch a browser, run a bounded agent task, and return
  typed agent history.
- `browser_use_session`: start, stop, and list persistent browser sessions.

`tools/list` includes both `inputSchema` and `outputSchema` for each tool.
`outputSchema` describes the JSON object returned in the call result's
`structuredContent` field.

Provider secrets are intentionally not part of MCP tool input schemas. A server
implementation reads provider credentials from its process environment or host
configuration. `browser_use_agent` accepts an optional `provider` input:
`openai-compatible` (default), `deepseek`, `groq`, `cerebras`, `mistral`,
`openrouter`, `vercel`, `anthropic`, `gemini`, or `ollama`.
OpenAI-compatible runs require `OPENAI_API_KEY` plus a tool `model` argument or
`OPENAI_MODEL`; `OPENAI_BASE_URL` is optional. DeepSeek, Groq, Cerebras,
Mistral, OpenRouter, and Vercel AI Gateway use the same Chat Completions wire
contract with provider-specific environment values: `DEEPSEEK_*`, `GROQ_*`,
`CEREBRAS_*`, `MISTRAL_*`, `OPENROUTER_*`, or `AI_GATEWAY_*` for API key,
model, and optional base URL. Vercel also accepts `VERCEL_OIDC_TOKEN` and
`VERCEL_MODEL`; DeepSeek, Cerebras, and Mistral use their upstream default
models when a tool `model` argument is omitted. OpenRouter also reads
`OPENROUTER_HTTP_REFERER`/`OPENROUTER_APP_URL` and
`OPENROUTER_X_TITLE`/`OPENROUTER_APP_TITLE` for app attribution headers.
DeepSeek structured-output
requests force a schema function tool call and parse the tool-call arguments,
while Cerebras uses prompt-only schema guidance. Anthropic runs require
`ANTHROPIC_API_KEY` plus a
tool `model` argument or `ANTHROPIC_MODEL`;
`ANTHROPIC_BASE_URL`, `ANTHROPIC_VERSION`, and `ANTHROPIC_MAX_TOKENS` are
optional. Gemini runs require `GEMINI_API_KEY` plus a tool `model` argument or
`GEMINI_MODEL`; `GEMINI_BASE_URL` is optional. Ollama runs require a tool
`model` argument or `OLLAMA_MODEL`; `OLLAMA_BASE_URL` or `OLLAMA_HOST` is
optional and defaults to `http://localhost:11434`.

For OpenAI-wire providers, `browser_use_agent` can override the provider
default structured-output strategy with an optional `structured_output_mode`
input. Supported values are `json-schema`, `json-object`, `prompt-only`, and
`tool-call`; aliases using underscores are accepted for JSON clients that avoid
kebab-case enum values.

`browser_use_agent` also accepts an optional typed `settings` object matching
`browser-use-core`'s `AgentSettings`, including upstream-compatible
`use_vision` values of `true`, `false`, or `"auto"`, image detail level, action
limits, step/LLM/per-action timeouts, final `done` responses after repeated
failures, optional `done.files_to_display` text expansion, prompt-history limits,
planning controls, upstream-style message compaction settings, thinking/flash
output modes, and prompt-visible DOM attributes plus initial actions,
upstream-compatible `directly_open_url` task URL auto-navigation,
`sample_images` prompt parts, excluded action names, conversation transcript
saving, non-fatal judge trace validation with optional `ground_truth`,
contract-preserved `generate_gif`, `calculate_cost`, and
`include_tool_call_examples` flags,
available file-path and sensitive-data prompt context, opt-in recent browser
events, and system-message override/extension fields.
On the final allowed `max_steps` step, `browser_use_agent` uses the same
done-only finalization contract as upstream browser-use so partial results are
returned through `done` instead of spending the last step on another browser
action. Non-final steps at or beyond 75% of the step budget also receive the
upstream budget-warning prompt so the model can consolidate before the final
step.
When `generate_gif` is enabled, successful agent runs write an agent-history
GIF from recorded screenshots to `agent_history.gif` or the provided path.
Token-cost accounting and tool-call example prompt side effects remain explicit
later runtime parity slices; the MCP schema accepts and round-trips the
upstream settings shape now.
Message compaction accepts `true`, `false`, `null`, or a settings object with
`compact_every_n_steps`, `trigger_char_count` or `trigger_token_count`,
`chars_per_token`, `keep_last_items`, `summary_max_chars`, and
`include_read_state`. Enabled runs compact older history into an unverified
`<compacted_memory>` prompt block and checkpoint field without failing the agent
when the summary request is unavailable.
Excluded action names are removed from the model output schema and rejected
before execution if a loose provider still returns one, while `done` remains
available for completion. The `screenshot` action is exposed only when
`use_vision` is `"auto"`; true keeps normal screenshot observations on, and
false keeps them off even after a loose screenshot action. Sensitive data values
are rendered to the model as placeholder names, not raw values, and placeholders
ending in `bu_2fa_code` generate TOTP codes at execution time. Provider
credentials remain environment-only and are intentionally absent from tool input
schemas.

Browser, replay, and agent tools support an optional `session_id` argument.
When omitted, the tool call uses a fresh one-shot browser. When present, the
stdio server reuses an in-process Chrome session for subsequent calls with the
same `session_id`, reconnects to an existing persistent record after restarts,
or creates a persistent record when the `session_id` is new and a URL is
supplied.
Use `browser_use_session` with `operation` set to `list` to inspect records,
`stop` to close and remove one, and `cleanup` to remove stale records. Cleanup
skips running sessions and unknown-liveness records by default; set `force` only
when a specific record should be stopped through normal stop semantics or
removed despite unknown liveness.
Persistent sessions created by the CLI are the same record format and can be
stopped through MCP, and MCP-created persistent sessions can be stopped with
`browser-use-rs session stop <id>`. Session records include a `status` field:
`running`, `stale`, `stopped`, or `unknown` depending on the recorded browser
process metadata.

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

Minimal HTTP daemon request shape:

```sh
curl -sS -X POST http://127.0.0.1:8765/rpc \
  -H 'content-type: application/json' \
  -H 'authorization: Bearer <token>' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```
