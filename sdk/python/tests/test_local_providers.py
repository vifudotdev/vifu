from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from vifu import vifu_mobile_ffi as native
from vifu.providers import LocalLlama, LocalWhisper, OpenAICompatible


class _CompletedRuntime:
    def __init__(self, output: dict[str, object]):
        self.output = output
        self.whisper_config = None
        self.llama_config = None
        self.invocation_data = None
        self.invocation_metadata = None

    def register_whisper_provider(self, _provider_id, config):
        self.whisper_config = config

    def register_llama_provider(self, _provider_id, config):
        self.llama_config = config

    def register_agent(self, *_args):
        return None

    def register_endpoint(self, *_args):
        return None

    def start_invoke(self, _endpoint, _session_id, data, metadata):
        self.invocation_data = data
        self.invocation_metadata = json.loads(metadata)
        return "handle"

    def take_invocation(self, _handle):
        result = mock.Mock()
        result.data = native.VifuInvocationData.JSON(json.dumps(self.output))
        result.metadata_json = "{}"
        result.state_json = "{}"
        poll = mock.Mock()
        poll.state = native.VifuInvocationState.COMPLETED
        poll.result = result
        poll.error = None
        return poll


class LocalProviderTests(unittest.TestCase):
    def test_openai_compatible_provider_is_code_configured_and_secret_safe(self) -> None:
        response = mock.MagicMock()
        response.read.return_value = json.dumps(
            {"choices": [{"message": {"role": "assistant", "content": "ok"}}]}
        ).encode()
        response.__enter__.return_value = response
        provider = OpenAICompatible(
            url="https://provider.example.com/openai/v1/chat/completions",
            model="gpt-test",
            api_key="private-token",
        )

        with mock.patch("vifu.providers.urllib.request.urlopen", return_value=response) as send:
            result = provider.complete(
                {
                    "messages": [{"role": "user", "content": "hello"}],
                    "stream": True,
                },
                session_id="call-1",
            )

        self.assertEqual(result["choices"][0]["message"]["content"], "ok")
        request = send.call_args.args[0]
        self.assertEqual(request.get_header("Authorization"), "Bearer private-token")
        body = json.loads(request.data)
        self.assertEqual(body["model"], "gpt-test")
        self.assertEqual(body["user"], "call-1")
        self.assertFalse(body["stream"])
        self.assertNotIn("private-token", repr(provider))
        self.assertNotIn("private-token", json.dumps(provider.vifu_settings))

    def test_local_llama_advertises_safe_accelerator_shutdown(self) -> None:
        self.assertTrue(LocalLlama.supports_safe_accelerator_shutdown)

    def test_local_whisper_invokes_the_embedded_binary_provider(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model = Path(directory) / "ggml-base.bin"
            model.write_bytes(b"model")
            runtime = _CompletedRuntime({"text": "こんにちは"})
            with mock.patch(
                "vifu.providers.native.VifuEmbeddedRuntime",
                return_value=runtime,
            ):
                provider = LocalWhisper(model=model, language="ja")
                text = provider.transcribe_wav(b"RIFF-wav", language="ja-JP")

        self.assertEqual(text, "こんにちは")
        self.assertEqual(runtime.whisper_config.model_path, str(model.resolve()))
        self.assertTrue(runtime.invocation_data.is_binary())
        self.assertEqual(runtime.invocation_data.bytes, b"RIFF-wav")
        self.assertEqual(
            runtime.invocation_metadata,
            {"binding": {"language": "ja"}},
        )

    def test_local_provider_reports_a_missing_model_before_startup(self) -> None:
        provider = LocalWhisper(model="missing-whisper-model.bin", language="ja")

        with self.assertRaisesRegex(FileNotFoundError, "missing-whisper-model.bin"):
            provider.prepare()

    def test_local_llama_converts_one_strands_tool_to_structured_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model = Path(directory) / "model.gguf"
            model.write_bytes(b"model")
            runtime = _CompletedRuntime(
                {
                    "choices": [
                        {
                            "message": {
                                "role": "assistant",
                                "content": '{"translation_zh":"你好"}',
                            },
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": 3,
                        "completion_tokens": 2,
                        "total_tokens": 5,
                    },
                }
            )
            with mock.patch(
                "vifu.providers.native.VifuEmbeddedRuntime",
                return_value=runtime,
            ):
                provider = LocalLlama(model=model)
                response = provider.complete(
                    {
                        "messages": [{"role": "user", "content": "translate"}],
                        "tools": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "publish_call_assist",
                                    "parameters": {
                                        "type": "object",
                                        "properties": {
                                            "translation_zh": {"type": "string"}
                                        },
                                        "required": ["translation_zh"],
                                    },
                                },
                            }
                        ],
                    },
                    session_id="call-1",
                )

        request = json.loads(runtime.invocation_data.json)
        self.assertEqual(
            request["response_format"]["json_schema"]["name"],
            "publish_call_assist",
        )
        tool_call = response["choices"][0]["message"]["tool_calls"][0]
        self.assertEqual(tool_call["function"]["name"], "publish_call_assist")
        self.assertEqual(
            json.loads(tool_call["function"]["arguments"]),
            {"translation_zh": "你好"},
        )

    def test_local_llama_uses_the_strands_tool_result_for_its_final_response(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model = Path(directory) / "model.gguf"
            model.write_bytes(b"model")
            runtime = _CompletedRuntime(
                {
                    "choices": [
                        {
                            "message": {
                                "role": "assistant",
                                "content": "The result was published.",
                            },
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": 8,
                        "completion_tokens": 4,
                        "total_tokens": 12,
                    },
                }
            )
            with mock.patch(
                "vifu.providers.native.VifuEmbeddedRuntime",
                return_value=runtime,
            ):
                provider = LocalLlama(model=model)
                response = provider.complete(
                    {
                        "messages": [
                            {"role": "user", "content": "Publish this."},
                            {
                                "role": "assistant",
                                "content": None,
                                "tool_calls": [
                                    {
                                        "id": "tool-1",
                                        "type": "function",
                                        "function": {
                                            "name": "publish_call_assist",
                                            "arguments": '{"translation_zh":"\u4f60\u597d"}',
                                        },
                                    }
                                ],
                            },
                            {
                                "role": "tool",
                                "tool_call_id": "tool-1",
                                "content": '{"published":true}',
                            },
                        ],
                        "tools": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "publish_call_assist",
                                    "parameters": {"type": "object"},
                                },
                            }
                        ],
                    },
                    session_id="call-1",
                )

        request = json.loads(runtime.invocation_data.json)
        self.assertNotIn("response_format", request)
        self.assertEqual(
            [message["role"] for message in request["messages"]],
            ["user", "assistant", "user"],
        )
        self.assertIn("publish_call_assist", request["messages"][1]["content"])
        self.assertIn("published", request["messages"][2]["content"])
        self.assertEqual(
            response["choices"][0]["message"]["content"],
            "The result was published.",
        )

    def test_local_llama_can_use_tools_again_on_a_later_user_turn(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            model = Path(directory) / "model.gguf"
            model.write_bytes(b"model")
            runtime = _CompletedRuntime(
                {
                    "choices": [
                        {
                            "message": {
                                "role": "assistant",
                                "content": '{"translation_zh":"\u518d\u89c1"}',
                            },
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {},
                }
            )
            with mock.patch(
                "vifu.providers.native.VifuEmbeddedRuntime",
                return_value=runtime,
            ):
                provider = LocalLlama(model=model)
                response = provider.complete(
                    {
                        "messages": [
                            {
                                "role": "tool",
                                "tool_call_id": "tool-1",
                                "content": '{"published":true}',
                            },
                            {"role": "user", "content": "Translate the next phrase."},
                        ],
                        "tools": [
                            {
                                "type": "function",
                                "function": {
                                    "name": "publish_call_assist",
                                    "parameters": {"type": "object"},
                                },
                            }
                        ],
                    },
                    session_id="call-2",
                )

        request = json.loads(runtime.invocation_data.json)
        self.assertIn("response_format", request)
        self.assertEqual(
            response["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "publish_call_assist",
        )


if __name__ == "__main__":
    unittest.main()
