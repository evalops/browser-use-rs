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
  [--auth-token <token>] [--pid-file /run/browser-use-rs.pid] \
  [--ready-file /run/browser-use-rs.ready.json]
browser-use-rs open <url>
browser-use-rs state <url> [--screenshot]
browser-use-rs screenshot <url> <output.png>
browser-use-rs click <url> <index>
browser-use-rs type <url> <index> <text>
browser-use-rs scroll <url> [--pages 1.0] [--down]
browser-use-rs actions <url> <actions.json> [--screenshot]
browser-use-rs replay <url> <history.json>
browser-use-rs agent <url> <task> --provider openai-compatible \
  [--api-key <key>] [--model <model>] [--base-url https://api.openai.com/v1] \
  [--allowed-domain example.com] [--prohibited-domain tracker.example.com] \
  [--block-ip-addresses] \
  [--max-steps 10] [--no-vision] [--max-actions-per-step 5] \
  [--no-final-response-after-failure] [--flash-mode] \
  [--include-attribute data-testid] [--available-file-path /tmp/report.pdf]
browser-use-rs agent <url> <task> --provider anthropic \
  [--api-key <key>] [--model <model>] [--base-url https://api.anthropic.com/v1] \
  [--max-steps 10]
browser-use-rs agent <url> <task> --provider gemini \
  [--api-key <key>] [--model <model>] \
  [--base-url https://generativelanguage.googleapis.com/v1beta] [--max-steps 10]
browser-use-rs agent <url> <task> --provider ollama \
  [--model <model>] [--base-url http://localhost:11434] [--max-steps 10]
browser-use-rs agent <url> <task> \
  --provider deepseek|groq|cerebras|mistral|openrouter|vercel \
  [--api-key <key>] [--model <model>] [--base-url <openai-compatible-url>] \
  [--structured-output-mode json-schema|json-object|prompt-only|tool-call] \
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

`replay` accepts serialized `AgentHistory` JSON, launches a fresh browser at
the supplied URL, captures current state, builds a rematched replay plan,
executes the guarded actions, and prints an `AgentHistoryReplayRun` containing
`current_state`, `plan`, and `execution`. Malformed history, state capture, and
rematch failures fail the command; action-level failures remain in the
serialized execution diagnostics.

Agent runs use the same one-shot browser lifecycle and print typed
`AgentHistory` JSON after the bounded run completes. The default provider is
`openai-compatible` and reads `OPENAI_API_KEY`, `OPENAI_MODEL`, and optional
`OPENAI_BASE_URL` from the environment. `--provider anthropic` reads
`ANTHROPIC_API_KEY`, `ANTHROPIC_MODEL`, optional `ANTHROPIC_BASE_URL`,
`ANTHROPIC_VERSION`, and `ANTHROPIC_MAX_TOKENS`. `--provider gemini` reads
`GEMINI_API_KEY`, `GEMINI_MODEL`, and optional `GEMINI_BASE_URL`.
`--provider ollama` reads `OLLAMA_MODEL` and optional `OLLAMA_BASE_URL` or
`OLLAMA_HOST`; it does not require an API key. The OpenAI-compatible upstream
provider aliases read `DEEPSEEK_API_KEY`/`DEEPSEEK_MODEL`, `GROQ_API_KEY`/
`GROQ_MODEL`, `CEREBRAS_API_KEY`/`CEREBRAS_MODEL`, `MISTRAL_API_KEY`/
`MISTRAL_MODEL`, `OPENROUTER_API_KEY`/`OPENROUTER_MODEL`, or
`AI_GATEWAY_API_KEY`/`AI_GATEWAY_MODEL` for Vercel AI Gateway. Vercel also
accepts `VERCEL_OIDC_TOKEN` and `VERCEL_MODEL`. Each alias has an optional
matching `*_BASE_URL` override. DeepSeek, Cerebras, and Mistral use their
upstream default model names when `--model` is omitted. CLI `--api-key`,
`--model`, and `--base-url` override the provider-specific environment values
where they apply. Structured-output requests use JSON Schema for OpenAI, Groq,
Mistral, OpenRouter, and Vercel AI Gateway, a forced schema function tool call
for DeepSeek, and prompt-only schema guidance for Cerebras. The OpenAI-compatible
adapter also supports `--structured-output-mode json-schema|json-object|prompt-only|tool-call`
to override an OpenAI-wire provider when the default mode does not fit the
selected model.

Agent runs accept repeated `--allowed-domain <pattern>` and
`--prohibited-domain <pattern>` flags plus `--block-ip-addresses` to enforce
browser-profile URL access policy on explicit navigation, post-navigation
redirect observations, navigation-capable action boundaries, and newly observed
tabs. Allowed domains take precedence over prohibited domains, matching upstream
browser-use. Supported patterns include exact hosts such as `example.com`,
wildcard hosts such as
`*.example.com`, scheme-specific URL prefixes such as `https://wiki.org`, and
URL globs such as `chrome-extension://*`.

Agent runs also expose the typed `AgentSettings` knobs used by the MCP agent
tool: `--no-vision`, `--max-failures`, `--max-actions-per-step`,
`--llm-timeout-seconds`, `--step-timeout-seconds`,
`--no-final-response-after-failure`, `--no-loop-detection`,
`--loop-detection-window`, `--no-thinking`, `--flash-mode`,
`--no-planning`, `--planning-replan-on-stall`,
`--planning-exploration-limit`, `--max-history-items`,
`--max-clickable-elements-length`, repeated `--include-attribute <name>` for
prompt-visible DOM attributes, and repeated `--available-file-path <path>` for
upstream-style file-path context in the agent prompt. Use repeated
`--sensitive-data <placeholder=value>` for global sensitive placeholders, and
repeated `--sensitive-data-domain <domain-pattern=placeholder=value>` for
domain-scoped placeholders. Sensitive values are replaced during action
execution while prompt context shows only placeholder names; placeholders whose
names end in `bu_2fa_code` are treated as TOTP seeds. Use
`--override-system-message` to replace the default system prompt or
`--extend-system-message` to append extra instructions to it. By default, when
repeated model/provider failures hit `--max-failures`, the agent makes one last
side-effect-free `done`-only model call so it can return any partial findings
with `success=false`.

`session` commands persist a local Chrome session across CLI invocations. The
session registry defaults to `~/.browser-use-rs/sessions` and can be overridden
with `BROWSER_USE_RS_STATE_DIR`. Session IDs may contain ASCII letters, digits,
`-`, and `_`. Session start/list/stop output includes a `status` field:
`running` when the recorded browser process is alive, `stale` when the recorded
process is gone, `stopped` after an explicit stop, and `unknown` when no process
id was recorded.

Use `browser-use-rs session cleanup` to remove records whose recorded browser
process is stale. It prints `cleaned_sessions` plus the remaining `sessions`.
By default cleanup skips running sessions and records with unknown liveness. Use
`browser-use-rs session cleanup <id> --force` only when you intentionally want
to force a specific running session through normal stop semantics, or remove a
record whose liveness cannot be established.

`mcp-stdio` runs a newline-delimited JSON-RPC MCP server over stdin/stdout. It
supports `initialize`, `ping`, `tools/list`, and `tools/call` for
`browser_use_state`, `browser_use_actions`, `browser_use_agent`, and
`browser_use_session`. MCP browser and agent tool inputs accept an optional
`session_id`; calls with the same `session_id` reuse the same in-process Chrome
session, reconnect to an existing persistent record after restarts, or create a
persistent record when the `session_id` is new and a URL is supplied. The MCP
`browser_use_agent` tool also accepts
`structured_output_mode` values `json-schema`, `json-object`, `prompt-only`,
and `tool-call`, matching the CLI override for OpenAI-wire provider fallbacks.
`browser_use_session` can start, stop, list, and clean up persistent session records. If
`session_id` matches a persistent session record, `mcp-stdio` reconnects to
that Chrome session even after the stdio server process restarts. Session list
output includes the same liveness `status` as the CLI registry.

`daemon` binds a local listener, prints the bound address on startup, shares one
in-process session runtime across active connections, and uses the same
persistent session registry as the CLI and MCP session tool. The default
`--transport tcp` exposes the same newline-delimited JSON-RPC surface as
`mcp-stdio` to each connection. `--transport http` exposes `GET /healthz` and
`POST /rpc`; the `/rpc` body is the same JSON-RPC request used by stdio/TCP and
returns the JSON-RPC response as JSON. `--auth-token <token>` or
`BROWSER_USE_RS_DAEMON_TOKEN=<token>` requires HTTP clients to send either
`Authorization: Bearer <token>` or `X-Browser-Use-Rs-Token: <token>`.
For supervised installs, `--pid-file <path>` writes the daemon process id after
the listener binds, and `--ready-file <path>` writes JSON with `ready`, `pid`,
`addr`, and `transport`. The daemon handles Ctrl-C/SIGINT/SIGTERM and removes
those lifecycle files on graceful shutdown. Packaged systemd and launchd
templates live under `packaging/`; see
[DAEMON_SUPERVISION.md](DAEMON_SUPERVISION.md) for install commands and
`/healthz` smokes.

## Current Limits

- DOM indexing is compact and accessibility-aware, including same-origin iframe
  traversal, open shadow-root traversal, AX role/name/state/value enrichment,
  backend/frontend node ids, and cached observed-node resolution for
  click/type/scroll/dropdown/upload actions, input
  mask/autocomplete/date-format hints, ARIA keyshortcut rendering, read-only
  state, plus duplicate long-attribute pruning and JavaScript click/pointer
  listener-backed control detection when Chrome exposes command-line inspection
  APIs. Full browser-use DOM/AX snapshot parity is still tracked separately.
- Agent runs currently support OpenAI-compatible Chat Completions plus
  DeepSeek, Groq, Cerebras, Mistral, OpenRouter, and Vercel AI Gateway aliases,
  Anthropic Messages, Gemini GenerateContent, and Ollama Chat structured-output
  adapters, with DeepSeek forced tool-call output, Cerebras prompt-only output,
  and explicit OpenAI-wire output-mode overrides for fallback paths.
- MCP tools are real over stdio and can reuse in-process sessions by
  `session_id`; new `session_id` calls with a URL create persistent records,
  and calls without `session_id` stay one-shot and ephemeral.
- History replay is currently exposed as a one-shot CLI command. Persistent
  session replay and MCP/daemon replay tools are tracked as follow-up surfaces.
- Persistent session `status` is a registry liveness hint, not a supervisor;
  stale records can be removed with `session cleanup`, but stale browser
  processes are not automatically restarted.
- The daemon is a local TCP or HTTP JSON-RPC surface with optional HTTP
  authentication, pid/ready files, and packaged systemd/launchd templates for
  external supervisors.
