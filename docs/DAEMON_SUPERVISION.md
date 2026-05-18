# Daemon Supervision

`browser-use-rs daemon` is meant to run as a local operator-owned service. The
daemon supports a TCP JSON-RPC transport and an HTTP JSON-RPC transport; the
packaged service templates use HTTP so supervisors and probes can call
`GET /healthz`.

The templates depend on the daemon lifecycle files:

- `--pid-file <path>` writes the daemon process id after the listener binds.
- `--ready-file <path>` writes JSON with `ready`, `pid`, `addr`, and
  `transport`.
- Ctrl-C, SIGINT, and SIGTERM remove those files on graceful shutdown.

`GET /healthz` is unauthenticated. `POST /rpc` requires
`BROWSER_USE_RS_DAEMON_TOKEN` when that environment variable is set.

Agent tool calls read provider credentials from the daemon process
environment. Add only the provider variables you use, for example
`OPENAI_API_KEY` and `OPENAI_MODEL`, or the Anthropic, Gemini, and Ollama
variables documented in [CLI.md](CLI.md).

## Systemd User Service

The packaged user unit lives at:

```text
packaging/systemd/browser-use-rs.service
```

It starts:

```sh
browser-use-rs daemon --transport http --addr 127.0.0.1:8765 \
  --pid-file "${XDG_RUNTIME_DIR}/browser-use-rs/daemon.pid" \
  --ready-file "${XDG_RUNTIME_DIR}/browser-use-rs/daemon.ready.json"
```

Default paths and environment:

```text
BROWSER_USE_RS_STATE_DIR=%h/.local/state/browser-use-rs
BROWSER_USE_RS_DAEMON_TOKEN=change-me
# optional for browser_use_agent with OpenAI-compatible models:
OPENAI_API_KEY=sk-change-me
OPENAI_MODEL=gpt-4.1-mini
binary=%h/.cargo/bin/browser-use-rs
pid=%t/browser-use-rs/daemon.pid
ready=%t/browser-use-rs/daemon.ready.json
```

Install and start it:

```sh
mkdir -p "$HOME/.config/systemd/user"
install -m 0644 packaging/systemd/browser-use-rs.service \
  "$HOME/.config/systemd/user/browser-use-rs.service"

# Edit the binary path and token before starting.
${EDITOR:-vi} "$HOME/.config/systemd/user/browser-use-rs.service"

systemctl --user daemon-reload
systemctl --user enable --now browser-use-rs.service
systemctl --user status browser-use-rs.service
```

Smoke the running daemon:

```sh
cat "${XDG_RUNTIME_DIR}/browser-use-rs/daemon.ready.json"
addr=$(jq -r .addr "${XDG_RUNTIME_DIR}/browser-use-rs/daemon.ready.json")
curl -fsS "http://${addr}/healthz"
```

Useful operations:

```sh
journalctl --user -u browser-use-rs.service -f
systemctl --user restart browser-use-rs.service
systemctl --user stop browser-use-rs.service
```

## Launchd Agent

The packaged launch agent lives at:

```text
packaging/launchd/com.evalops.browser-use-rs.plist
```

It starts:

```sh
browser-use-rs daemon --transport http --addr 127.0.0.1:8765 \
  --pid-file "$HOME/Library/Caches/browser-use-rs/daemon.pid" \
  --ready-file "$HOME/Library/Caches/browser-use-rs/daemon.ready.json"
```

Default paths and environment:

```text
BROWSER_USE_RS_STATE_DIR=/Users/YOU/Library/Application Support/browser-use-rs
BROWSER_USE_RS_DAEMON_TOKEN=change-me
# optional for browser_use_agent with OpenAI-compatible models:
OPENAI_API_KEY=sk-change-me
OPENAI_MODEL=gpt-4.1-mini
binary=/Users/YOU/.cargo/bin/browser-use-rs
pid=/Users/YOU/Library/Caches/browser-use-rs/daemon.pid
ready=/Users/YOU/Library/Caches/browser-use-rs/daemon.ready.json
stdout=/Users/YOU/Library/Logs/browser-use-rs/daemon.stdout.log
stderr=/Users/YOU/Library/Logs/browser-use-rs/daemon.stderr.log
```

Install and start it:

```sh
mkdir -p "$HOME/Library/LaunchAgents" \
  "$HOME/Library/Caches/browser-use-rs" \
  "$HOME/Library/Logs/browser-use-rs" \
  "$HOME/Library/Application Support/browser-use-rs"
install -m 0644 packaging/launchd/com.evalops.browser-use-rs.plist \
  "$HOME/Library/LaunchAgents/com.evalops.browser-use-rs.plist"

# Replace /Users/YOU, the binary path, and the token before starting.
${EDITOR:-vi} "$HOME/Library/LaunchAgents/com.evalops.browser-use-rs.plist"
plutil -lint "$HOME/Library/LaunchAgents/com.evalops.browser-use-rs.plist"

launchctl bootstrap "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/com.evalops.browser-use-rs.plist"
launchctl enable "gui/$(id -u)/com.evalops.browser-use-rs"
launchctl kickstart -k "gui/$(id -u)/com.evalops.browser-use-rs"
launchctl print "gui/$(id -u)/com.evalops.browser-use-rs"
```

Smoke the running daemon:

```sh
cat "$HOME/Library/Caches/browser-use-rs/daemon.ready.json"
addr=$(jq -r .addr "$HOME/Library/Caches/browser-use-rs/daemon.ready.json")
curl -fsS "http://${addr}/healthz"
```

Useful operations:

```sh
launchctl kickstart -k "gui/$(id -u)/com.evalops.browser-use-rs"
launchctl bootout "gui/$(id -u)" \
  "$HOME/Library/LaunchAgents/com.evalops.browser-use-rs.plist"
tail -f "$HOME/Library/Logs/browser-use-rs/daemon.stderr.log"
```

## Local Command Smoke

This smoke uses the same command shape as the templates without installing a
supervisor:

```sh
cargo build -p browser-use-cli
tmp=$(mktemp -d)
./target/debug/browser-use-rs daemon --transport http --addr 127.0.0.1:0 \
  --pid-file "$tmp/daemon.pid" \
  --ready-file "$tmp/daemon.ready.json" >"$tmp/stdout" 2>"$tmp/stderr" &
daemon_pid=$!

for _ in $(seq 1 50); do
  test -s "$tmp/daemon.ready.json" && break
  sleep 0.1
done

if [ ! -s "$tmp/daemon.ready.json" ]; then
  cat "$tmp/stderr"
  kill "$daemon_pid" 2>/dev/null || true
  wait "$daemon_pid" 2>/dev/null || true
  exit 1
fi

addr=$(jq -r .addr "$tmp/daemon.ready.json")
curl -fsS "http://${addr}/healthz"
kill -TERM "$daemon_pid"
wait "$daemon_pid"
test ! -e "$tmp/daemon.pid"
test ! -e "$tmp/daemon.ready.json"
rm -rf "$tmp"
```
