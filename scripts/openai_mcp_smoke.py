#!/usr/bin/env python3
"""No-secret MCP smoke test for OpenAI strict schema transport."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE = (
    ROOT / "crates" / "browser-use-conformance" / "fixtures" / "openai_agent_output_schema.json"
)
DONE_TEXT = "saw Offline MCP Smoke answer 42"
UNSUPPORTED_SCHEMA_KEYWORDS = {
    "allOf",
    "const",
    "default",
    "dependentRequired",
    "dependentSchemas",
    "else",
    "format",
    "if",
    "maximum",
    "maxItems",
    "maxLength",
    "minimum",
    "minItems",
    "minLength",
    "multipleOf",
    "not",
    "oneOf",
    "pattern",
    "patternProperties",
    "then",
}


def lint_openai_strict_schema(schema: Any) -> list[str]:
    """Return OpenAI strict-schema compatibility errors for a JSON schema."""
    errors: list[str] = []

    if not isinstance(schema, dict):
        return ["$ must be an object schema"]
    if "anyOf" in schema:
        errors.append("$ must not use anyOf at the root")
    if not is_object_schema(schema):
        errors.append("$ must be an object schema")

    walk_schema(schema, "$", errors, inside_any_of_variant=False)
    return errors


def walk_schema(
    value: Any,
    path: str,
    errors: list[str],
    *,
    inside_any_of_variant: bool,
) -> None:
    if isinstance(value, bool):
        errors.append(f"{path} must not use boolean schemas")
        return
    if isinstance(value, list):
        for index, item in enumerate(value):
            walk_schema(item, f"{path}[{index}]", errors, inside_any_of_variant=False)
        return
    if not isinstance(value, dict):
        return

    if "definitions" in value:
        errors.append(f"{path}.definitions must be rewritten to $defs")

    for key in sorted(UNSUPPORTED_SCHEMA_KEYWORDS):
        if key in value:
            errors.append(f"{path}.{key} is unsupported by OpenAI strict schemas")

    if ref := value.get("$ref"):
        if isinstance(ref, str) and ref.startswith("#/definitions/"):
            errors.append(f"{path} must rewrite #/definitions refs to #/$defs")

    if is_object_schema(value):
        validate_strict_object(value, path, errors)

    if inside_any_of_variant:
        validate_any_of_variant_properties(value, path, errors)

    for key, child in value.items():
        if key == "additionalProperties":
            continue
        if key == "anyOf" and isinstance(child, list):
            for index, variant in enumerate(child):
                walk_schema(
                    variant,
                    f"{path}.anyOf[{index}]",
                    errors,
                    inside_any_of_variant=True,
                )
            continue
        child_path = f"{path}.{key}"
        if key == "properties" and isinstance(child, dict):
            for property_name, property_schema in child.items():
                walk_schema(
                    property_schema,
                    f"{child_path}.{property_name}",
                    errors,
                    inside_any_of_variant=False,
                )
            continue
        walk_schema(child, child_path, errors, inside_any_of_variant=False)


def is_object_schema(value: dict[str, Any]) -> bool:
    if "properties" in value:
        return True
    schema_type = value.get("type")
    if schema_type == "object":
        return True
    return isinstance(schema_type, list) and "object" in schema_type


def validate_strict_object(value: dict[str, Any], path: str, errors: list[str]) -> None:
    if value.get("additionalProperties") is not False:
        errors.append(f"{path}.additionalProperties must be false")

    properties = value.get("properties")
    if not isinstance(properties, dict):
        return

    property_keys = sorted(properties)
    required = value.get("required")
    if not isinstance(required, list) or sorted(required) != property_keys:
        errors.append(f"{path}.required must include exactly property keys {property_keys}")


def validate_any_of_variant_properties(
    value: dict[str, Any], path: str, errors: list[str]
) -> None:
    properties = value.get("properties")
    if not isinstance(properties, dict):
        return
    for property_name, property_schema in properties.items():
        if not isinstance(property_schema, dict):
            continue
        ref = property_schema.get("$ref")
        if isinstance(ref, str) and ref.startswith("#/$defs/"):
            errors.append(
                f"{path}.properties.{property_name} "
                "must inline #/$defs refs inside anyOf variants"
            )


def extract_strict_schema(payload: dict[str, Any]) -> dict[str, Any]:
    response_format = payload.get("response_format")
    if isinstance(response_format, dict) and response_format.get("type") == "json_schema":
        json_schema = response_format.get("json_schema")
        if isinstance(json_schema, dict) and json_schema.get("strict") is True:
            schema = json_schema.get("schema")
            if isinstance(schema, dict):
                return schema

    tools = payload.get("tools")
    if isinstance(tools, list):
        for tool in tools:
            if not isinstance(tool, dict):
                continue
            function = tool.get("function")
            if isinstance(function, dict) and function.get("strict") is True:
                parameters = function.get("parameters")
                if isinstance(parameters, dict):
                    return parameters

    raise ValueError("request did not include a strict OpenAI schema")


def schema_kind(schema: dict[str, Any]) -> str:
    properties = schema.get("properties")
    if not isinstance(properties, dict):
        return "unknown"
    if "action" in properties:
        return "agent_output"
    if "verdict" in properties:
        return "judgement"
    if "summary" in properties:
        return "message_compaction"
    return "unknown"


def output_for_schema(schema: dict[str, Any], done_text: str) -> dict[str, Any]:
    kind = schema_kind(schema)
    if kind == "agent_output":
        return {
            "thinking": None,
            "evaluation_previous_goal": None,
            "memory": "offline smoke observed the page",
            "next_goal": None,
            "current_plan_item": None,
            "plan_update": None,
            "action": [
                {
                    "done": {
                        "text": done_text,
                        "success": True,
                        "files_to_display": [],
                    }
                }
            ],
        }
    if kind == "judgement":
        return {
            "reasoning": "offline mock judge accepted the exact done text",
            "verdict": True,
            "failure_reason": None,
            "impossible_task": False,
            "reached_captcha": False,
        }
    if kind == "message_compaction":
        return {"summary": "offline smoke summary"}
    return {}


def mock_chat_completion(payload: dict[str, Any], *, done_text: str = DONE_TEXT) -> dict[str, Any]:
    schema = extract_strict_schema(payload)
    errors = lint_openai_strict_schema(schema)
    if errors:
        raise ValueError("; ".join(errors))

    model = str(payload.get("model") or "offline-openai")
    output = output_for_schema(schema, done_text)
    encoded_output = json.dumps(output, separators=(",", ":"))
    tools = payload.get("tools")

    message: dict[str, Any]
    finish_reason: str
    if isinstance(tools, list) and tools:
        function = tools[0].get("function") if isinstance(tools[0], dict) else {}
        function_name = (
            function.get("name")
            if isinstance(function, dict) and isinstance(function.get("name"), str)
            else "agent_output"
        )
        message = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_offline_openai_schema_smoke",
                    "type": "function",
                    "function": {
                        "name": function_name,
                        "arguments": encoded_output,
                    },
                }
            ],
        }
        finish_reason = "tool_calls"
    else:
        message = {"role": "assistant", "content": encoded_output, "refusal": None}
        finish_reason = "stop"

    return {
        "id": f"chatcmpl-offline-{time.time_ns()}",
        "object": "chat.completion",
        "created": int(time.time()),
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": message,
                "logprobs": None,
                "finish_reason": finish_reason,
            }
        ],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2,
        },
    }


class OpenAiMockServer(ThreadingHTTPServer):
    done_text: str
    payloads: list[dict[str, Any]]
    schemas: list[dict[str, Any]]


class OpenAiMockHandler(BaseHTTPRequestHandler):
    server: OpenAiMockServer

    def do_POST(self) -> None:
        if self.path != "/v1/chat/completions":
            self.write_json(404, {"error": {"message": f"unexpected path {self.path}"}})
            return

        length = int(self.headers.get("content-length", "0"))
        try:
            payload = json.loads(self.rfile.read(length))
            if not isinstance(payload, dict):
                raise ValueError("request body must be a JSON object")
            self.server.payloads.append(payload)
            self.server.schemas.append(extract_strict_schema(payload))
            response = mock_chat_completion(payload, done_text=self.server.done_text)
        except Exception as error:  # noqa: BLE001 - HTTP test server reports validation errors.
            self.write_json(
                400,
                {
                    "error": {
                        "message": str(error),
                        "type": "invalid_request_error",
                        "param": "response_format",
                        "code": None,
                    }
                },
            )
            return

        self.write_json(200, response)

    def write_json(self, status: int, value: dict[str, Any]) -> None:
        body = json.dumps(value).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: Any) -> None:
        return


def start_mock_server(done_text: str) -> tuple[OpenAiMockServer, threading.Thread]:
    server = OpenAiMockServer(("127.0.0.1", 0), OpenAiMockHandler)
    server.done_text = done_text
    server.payloads = []
    server.schemas = []
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread


def run_mcp_agent_smoke(
    *,
    base_url: str,
    done_text: str,
    command: list[str],
    timeout_seconds: int,
) -> dict[str, Any]:
    request_lines = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "offline-openai-schema-smoke", "version": "0"},
            },
        },
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "browser_use_agent",
                "arguments": {
                    "provider": "openai",
                    "model": "offline-openai-schema-model",
                    "base_url": base_url,
                    "url": (
                        "data:text/html,<html><head><title>Offline%20MCP%20Smoke</title>"
                        "</head><body><main><h1>Offline MCP Smoke</h1>"
                        "<p id=answer>The answer is 42.</p></main></body></html>"
                    ),
                    "task": (
                        "Inspect the current page. Do not click or navigate. "
                        f"Make done your first browser action, with text exactly: {done_text}"
                    ),
                    "max_steps": 3,
                    "settings": {"use_vision": False},
                },
            },
        },
    ]
    stdin = "\n".join(json.dumps(line) for line in request_lines) + "\n"
    env = os.environ.copy()
    env["OPENAI_API_KEY"] = "offline-test-key"
    env.pop("OPENAI_BASE_URL", None)
    env.pop("OPENAI_MODEL", None)

    completed = subprocess.run(
        command,
        input=stdin,
        text=True,
        capture_output=True,
        cwd=ROOT,
        env=env,
        timeout=timeout_seconds,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"MCP command exited {completed.returncode}\nSTDOUT:\n{completed.stdout}\nSTDERR:\n{completed.stderr}"
        )

    responses = [json.loads(line) for line in completed.stdout.splitlines() if line.strip()]
    if len(responses) != 2:
        raise RuntimeError(f"expected 2 JSON-RPC responses, got {len(responses)}")
    agent_response = next((response for response in responses if response.get("id") == 2), None)
    if not isinstance(agent_response, dict):
        raise RuntimeError("missing browser_use_agent response")
    if "error" in agent_response:
        raise RuntimeError(f"browser_use_agent JSON-RPC error: {agent_response['error']}")

    result = agent_response.get("result")
    if not isinstance(result, dict):
        raise RuntimeError("browser_use_agent result was not an object")
    if result.get("isError"):
        raise RuntimeError(f"browser_use_agent reported isError: {result}")

    history = (
        result.get("structuredContent", {})
        .get("history", {})
        .get("items", [])
    )
    all_errors = [
        action_result["error"]
        for item in history
        for action_result in item.get("result", [])
        if action_result.get("error") is not None
    ]
    if all_errors:
        raise RuntimeError(f"agent history contained provider/action errors: {all_errors}")
    if not history:
        raise RuntimeError("agent history was empty")

    final_results = history[-1].get("result", [])
    final_done = next(
        (
            action_result
            for action_result in final_results
            if action_result.get("is_done") is True
        ),
        None,
    )
    if not final_done:
        raise RuntimeError(f"final result was not done: {final_results}")
    if final_done.get("extracted_content") != done_text:
        raise RuntimeError(f"unexpected done text: {final_done.get('extracted_content')!r}")

    return {
        "responses": responses,
        "item_count": len(history),
        "done_text": final_done.get("extracted_content"),
        "all_errors": all_errors,
    }


def agent_output_schema(schemas: list[dict[str, Any]]) -> dict[str, Any]:
    for schema in schemas:
        if schema_kind(schema) == "agent_output":
            return schema
    raise RuntimeError("fake OpenAI server did not receive an AgentOutput schema")


def canonical_json(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True) + "\n"


def verify_or_update_fixture(
    schema: dict[str, Any],
    fixture_path: Path,
    *,
    update_fixture: bool,
) -> None:
    rendered = canonical_json(schema)
    if update_fixture:
        fixture_path.parent.mkdir(parents=True, exist_ok=True)
        fixture_path.write_text(rendered)
        return
    expected = fixture_path.read_text()
    if rendered != expected:
        raise RuntimeError(
            f"{fixture_path} is out of date; run scripts/openai_mcp_smoke.py --update-fixture"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update-fixture", action="store_true")
    parser.add_argument("--fixture", type=Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--timeout-seconds", type=int, default=180)
    parser.add_argument("--attempts", type=int, default=2)
    parser.add_argument(
        "--command",
        nargs=argparse.REMAINDER,
        default=["cargo", "run", "-q", "-p", "browser-use-cli", "--", "mcp-stdio"],
        help="Command used to launch the MCP stdio server; default runs the local CLI crate.",
    )
    args = parser.parse_args()
    if not args.command:
        parser.error("--command requires at least one argument")
    if args.attempts < 1:
        parser.error("--attempts must be at least 1")

    server, thread = start_mock_server(DONE_TEXT)
    try:
        host, port = server.server_address
        base_url = f"http://{host}:{port}/v1"
        summary = None
        last_error = None
        for attempt in range(1, args.attempts + 1):
            try:
                summary = run_mcp_agent_smoke(
                    base_url=base_url,
                    done_text=DONE_TEXT,
                    command=args.command,
                    timeout_seconds=args.timeout_seconds,
                )
                break
            except Exception as error:  # noqa: BLE001 - bounded smoke retry reports final error.
                last_error = error
                server.payloads.clear()
                server.schemas.clear()
                if attempt == args.attempts:
                    raise
                print(
                    f"offline MCP smoke attempt {attempt}/{args.attempts} failed: {error}",
                    file=sys.stderr,
                )
                time.sleep(5)
        if summary is None:
            raise RuntimeError(f"offline MCP smoke did not run: {last_error}")
        schema = agent_output_schema(server.schemas)
        verify_or_update_fixture(
            schema,
            args.fixture,
            update_fixture=args.update_fixture,
        )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)

    try:
        fixture_display = str(args.fixture.resolve().relative_to(ROOT))
    except ValueError:
        fixture_display = str(args.fixture)

    print(
        json.dumps(
            {
                "ok": True,
                "mock_requests": len(server.payloads),
                "item_count": summary["item_count"],
                "done_text": summary["done_text"],
                "all_errors": summary["all_errors"],
                "fixture": fixture_display,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
