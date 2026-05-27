import unittest

from scripts.openai_mcp_smoke import (
    extract_strict_schema,
    lint_openai_strict_schema,
    mock_chat_completion,
)


class OpenAiStrictSchemaLintTests(unittest.TestCase):
    def test_accepts_normalized_action_union_schema(self) -> None:
        schema = {
            "type": "object",
            "additionalProperties": False,
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "additionalProperties": False,
                                "required": ["find_elements"],
                                "properties": {
                                    "find_elements": {
                                        "type": "object",
                                        "additionalProperties": False,
                                        "required": ["selector"],
                                        "properties": {
                                            "selector": {"type": "string"},
                                        },
                                    },
                                },
                            }
                        ]
                    },
                },
            },
        }

        self.assertEqual(lint_openai_strict_schema(schema), [])

    def test_rejects_action_union_variant_property_refs(self) -> None:
        schema = {
            "type": "object",
            "additionalProperties": False,
            "required": ["action"],
            "properties": {
                "action": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {
                                "type": "object",
                                "additionalProperties": False,
                                "required": ["find_elements"],
                                "properties": {
                                    "find_elements": {
                                        "$ref": "#/$defs/FindElementsAction",
                                    },
                                },
                            }
                        ]
                    },
                },
            },
            "$defs": {
                "FindElementsAction": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["selector"],
                    "properties": {"selector": {"type": "string"}},
                },
            },
        }

        errors = lint_openai_strict_schema(schema)

        self.assertIn(
            "$.properties.action.items.anyOf[0].properties.find_elements "
            "must inline #/$defs refs inside anyOf variants",
            errors,
        )

    def test_rejects_non_strict_object_keywords(self) -> None:
        schema = {
            "type": "object",
            "required": ["ok", "extra"],
            "properties": {
                "ok": {"type": "string", "default": "yes"},
                "freeform": True,
            },
            "definitions": {},
        }

        errors = lint_openai_strict_schema(schema)

        self.assertIn("$.additionalProperties must be false", errors)
        self.assertIn(
            "$.required must include exactly property keys ['freeform', 'ok']", errors
        )
        self.assertIn("$.properties.ok.default is unsupported by OpenAI strict schemas", errors)
        self.assertIn("$.properties.freeform must not use boolean schemas", errors)
        self.assertIn("$.definitions must be rewritten to $defs", errors)


class OpenAiMockResponseTests(unittest.TestCase):
    def test_extracts_response_format_schema(self) -> None:
        payload = {
            "response_format": {
                "type": "json_schema",
                "json_schema": {"strict": True, "schema": {"type": "object"}},
            }
        }

        self.assertEqual(extract_strict_schema(payload), {"type": "object"})

    def test_returns_agent_done_json_for_agent_output_schema(self) -> None:
        response = mock_chat_completion(
            {
                "model": "offline-test",
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "strict": True,
                        "schema": {
                            "type": "object",
                            "additionalProperties": False,
                            "required": ["action"],
                            "properties": {"action": {"type": "array"}},
                        },
                    },
                },
            },
            done_text="saw Offline MCP Smoke answer 42",
        )

        content = response["choices"][0]["message"]["content"]

        self.assertIn('"action"', content)
        self.assertIn("saw Offline MCP Smoke answer 42", content)

    def test_returns_tool_call_arguments_for_tool_payloads(self) -> None:
        response = mock_chat_completion(
            {
                "model": "offline-test",
                "tools": [
                    {
                        "type": "function",
                        "function": {
                            "name": "agent_output",
                            "strict": True,
                            "parameters": {
                                "type": "object",
                                "additionalProperties": False,
                                "required": ["action"],
                                "properties": {"action": {"type": "array"}},
                            },
                        },
                    }
                ],
            },
            done_text="saw Offline MCP Smoke answer 42",
        )

        tool_call = response["choices"][0]["message"]["tool_calls"][0]

        self.assertEqual(tool_call["function"]["name"], "agent_output")
        self.assertIn("saw Offline MCP Smoke answer 42", tool_call["function"]["arguments"])


if __name__ == "__main__":
    unittest.main()
