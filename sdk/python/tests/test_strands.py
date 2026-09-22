from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from vifu.integrations.strands import (
    _ModelTarget,
    _openai_request,
    _post_json,
    _strands_events,
    openai_compatible_model,
)


class VifuStrandsTests(unittest.TestCase):
    def test_direct_openai_compatible_target_does_not_need_a_dashboard_profile(self) -> None:
        request = SimpleNamespace(session_id="conversation-8")

        target = _ModelTarget.for_openai_compatible(
            request,
            url="http://127.0.0.1:11434/v1/chat/completions",
            model="qwen2.5:7b",
            api_key="local-provider-secret",
        )

        self.assertEqual(target.url, "http://127.0.0.1:11434/v1/chat/completions")
        self.assertEqual(target.profile, "qwen2.5:7b")
        self.assertEqual(target.invocation_id, "conversation-8")
        self.assertNotIn("local-provider-secret", repr(target))

    def test_direct_openai_compatible_target_rejects_plaintext_remote_urls(self) -> None:
        with self.assertRaisesRegex(ValueError, "HTTPS"):
            _ModelTarget.for_openai_compatible(
                SimpleNamespace(session_id="conversation-9"),
                url="http://provider.example/v1/chat/completions",
                model="example-model",
            )

    def test_direct_openai_compatible_target_rejects_non_http_loopback_urls(self) -> None:
        with self.assertRaisesRegex(ValueError, "HTTPS"):
            _ModelTarget.for_openai_compatible(
                SimpleNamespace(session_id="conversation-10"),
                url="ftp://localhost/v1/chat/completions",
                model="example-model",
            )

    @unittest.skipUnless(
        importlib.util.find_spec("strands") is not None,
        "install the strands extra to run the framework compatibility test",
    )
    def test_public_model_factory_implements_the_installed_strands_contract(self) -> None:
        from strands import Agent
        from strands.models import Model

        model = openai_compatible_model(
            SimpleNamespace(session_id="conversation-11"),
            url="http://127.0.0.1:11434/v1/chat/completions",
            model="example-model",
        )

        self.assertIsInstance(model, Model)
        self.assertEqual(model.get_config()["model_id"], "example-model")
        with mock.patch(
            "vifu.integrations.strands._post_json",
            return_value={
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "Hello from the Vifu Provider.",
                        },
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 4,
                    "completion_tokens": 6,
                    "total_tokens": 10,
                },
            },
        ) as complete:
            result = Agent(model=model)("Hello")

        self.assertIn("Hello from the Vifu Provider.", str(result))
        complete.assert_called_once()

    def test_local_target_uses_the_current_app_and_profile(self) -> None:
        app = SimpleNamespace(
            server_url="http://127.0.0.1:6790",
            slug="call-assist",
        )
        request = SimpleNamespace(session_id="conversation-7")
        with mock.patch.dict("os.environ", {}, clear=True):
            target = _ModelTarget.for_app(app, request, "phone-translator")

        self.assertEqual(
            target.url,
            "http://127.0.0.1:6790/call-assist/v1/chat/completions",
        )
        self.assertEqual(target.invocation_id, "conversation-7")
        self.assertIsNone(target.token)

    def test_managed_target_uses_an_invocation_scoped_credential(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "provider.json"
            path.write_text(
                json.dumps(
                    {
                        "url": "https://api.example/v1/vifu/inference",
                        "token": "short-lived-token",
                        "invocationId": "invocation-123",
                        "expiresAt": "2030-01-01T00:00:00Z",
                    }
                )
            )
            with mock.patch.dict(
                "os.environ",
                {"VIFU_PROVIDER_CREDENTIAL_FILE": str(path)},
                clear=True,
            ):
                target = _ModelTarget.for_app(
                    SimpleNamespace(),
                    SimpleNamespace(),
                    "phone-translator",
                )

        self.assertEqual(target.profile, "phone-translator")
        self.assertEqual(target.invocation_id, "invocation-123")
        self.assertNotIn("short-lived-token", repr(target))

    def test_converts_strands_tools_to_openai_messages(self) -> None:
        target = _ModelTarget(
            url="http://127.0.0.1:6790/app/v1/chat/completions",
            profile="phone-translator",
            invocation_id="conversation-7",
        )
        payload = _openai_request(
            target=target,
            system_prompt="Use the publish tool.",
            messages=[
                {"role": "user", "content": [{"text": "translate this"}]},
                {
                    "role": "assistant",
                    "content": [
                        {
                            "toolUse": {
                                "toolUseId": "tool-1",
                                "name": "publish_call_assist",
                                "input": {"translation_zh": "你好"},
                            }
                        }
                    ],
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "toolResult": {
                                "toolUseId": "tool-1",
                                "content": [{"json": {"published": True}}],
                            }
                        }
                    ],
                },
            ],
            tool_specs=[
                {
                    "name": "publish_call_assist",
                    "description": "Publish a visual card",
                    "inputSchema": {"json": {"type": "object", "properties": {}}},
                }
            ],
            temperature=0.2,
            max_tokens=1_200,
        )

        self.assertEqual(payload["model"], "phone-translator")
        self.assertEqual(payload["user"], "conversation-7")
        self.assertEqual(payload["messages"][3]["role"], "tool")
        self.assertEqual(payload["tools"][0]["function"]["name"], "publish_call_assist")

    def test_converts_openai_tool_call_to_strands_events(self) -> None:
        events = _strands_events(
            {
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": None,
                            "tool_calls": [
                                {
                                    "id": "tool-1",
                                    "function": {
                                        "name": "publish_call_assist",
                                        "arguments": json.dumps({"translation_zh": "你好"}),
                                    },
                                }
                            ],
                        },
                        "finish_reason": "tool_calls",
                    }
                ],
                "usage": {"prompt_tokens": 10, "completion_tokens": 4},
            },
            latency_ms=25,
        )

        self.assertEqual(events[1]["contentBlockStart"]["start"]["toolUse"]["name"], "publish_call_assist")
        self.assertEqual(events[4], {"messageStop": {"stopReason": "tool_use"}})
        self.assertEqual(events[5]["metadata"]["usage"]["totalTokens"], 14)

    def test_provider_request_identifies_the_vifu_python_sdk(self) -> None:
        target = _ModelTarget(
            url="https://api.example/v1/chat/completions",
            profile="phone-translator",
            invocation_id="conversation-7",
            token="short-lived-token",
        )
        response = mock.MagicMock()
        response.read.return_value = b"{}"
        context = mock.MagicMock()
        context.__enter__.return_value = response

        with mock.patch("vifu.integrations.strands.urllib.request.urlopen", return_value=context) as post:
            self.assertEqual(_post_json(target, {"messages": []}), {})

        request = post.call_args.args[0]
        self.assertEqual(request.get_header("User-agent"), "Vifu-Python-SDK/0.1.11")
        self.assertEqual(request.get_header("Authorization"), "Bearer short-lived-token")


if __name__ == "__main__":
    unittest.main()
