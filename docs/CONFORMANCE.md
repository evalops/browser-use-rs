# Conformance Plan

The project is successful only when behavior is demonstrably compatible with
browser-use where compatibility is claimed.

## Inputs

- Upstream source at `933e28c599ddd74c15a48568f159da95547e40dd`.
- Upstream docs for quickstart, CLI, browser configuration, custom tools, and
  supported models.
- Upstream test intent from `tests/ci` and task fixtures.
- Local deterministic HTML fixtures for browser, DOM, and action behavior.

## Test Families

1. Schema snapshots: action JSON schemas, browser state JSON, and agent output.
2. DOM fixtures: numbered clickable elements, text representation, selector
   maps, iframes, hidden elements, dropdowns, ARIA widget roles, and
   accessibility names.
3. Browser actions: navigation, search, click, input, scroll, keyboard, tab
   switching, downloads, screenshots, and PDF output.
4. Agent loop: max steps, max failures, multi-action aborts after navigation,
   loop nudges, planning fields, done semantics, and final history.
5. Provider contracts: OpenAI-compatible, Anthropic, and Gemini
   structured-output payloads first, then local/Ollama and generic HTTP
   adapters as compatibility expands.
6. CLI/MCP: persistent session lifecycle, JSON output stability, and error
   shapes.

## Drift Policy

Upstream bumps must include:

- Old and new upstream commit SHAs.
- A summary of changed contracts.
- Updated conformance fixtures or explicit deferred gaps.
- A changelog entry describing compatibility impact.

## Current Fixtures

- `simple_interactive_state.json`: compact DOM text and selector-map fixture.
- `mixed_interactive_state.json`: selector-map fixture for accessible labels,
  attributes, bounds, dropdown current values, and scrollable metadata.
- `simple_action_sequence.json`: typed browser action sequence fixture.
- `simple_action_results.json`: expected action-result fixture for the action
  sequence harness.
- `simple_agent_history.json`: deterministic scripted-agent replay fixture
  covering schema-guided model output, previous-result prompt context, browser
  action execution, `done`, and serialized history.
