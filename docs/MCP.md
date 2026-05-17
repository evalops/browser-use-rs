# MCP

`browser-use-mcp` owns the stable JSON tool contracts that the MCP server will
expose.

Current tool contracts:

- `browser_use_state`: launch a browser, navigate to a URL, and return browser
  state.
- `browser_use_actions`: launch a browser, execute a JSON array of
  `BrowserAction` values, and return action results plus final state.
- `browser_use_agent`: launch a browser, run a bounded agent task, and return
  typed agent history.

Provider secrets are intentionally not part of MCP tool input schemas. A server
implementation should read provider credentials from its process environment or
host configuration.

To inspect the manifest from the CLI:

```sh
browser-use-rs mcp-tools
```
