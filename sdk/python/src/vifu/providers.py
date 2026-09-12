"""Code-configured Providers that run inside a Python Vifu App."""

from __future__ import annotations

import json
import threading
import time
import uuid
from pathlib import Path
from typing import Any, Mapping

from . import vifu_mobile_ffi as native

_DEFAULT_TIMEOUT_SECONDS = 70.0


class LocalProviderError(RuntimeError):
    """An in-process Vifu Provider could not start or complete an invocation."""


class LocalWhisper:
    """Resident whisper.cpp transcription Provider backed by the Vifu Runtime."""

    provider = "vifu-local-whisper"

    def __init__(
        self,
        *,
        model: str | Path,
        language: str | None = None,
        timeout: float = _DEFAULT_TIMEOUT_SECONDS,
    ) -> None:
        self.model = str(model)
        self.language = _language(language)
        self.timeout = _positive_timeout(timeout)
        self._model_path: Path | None = None
        self._runtime: native.VifuEmbeddedRuntime | None = None
        self._lock = threading.RLock()

    def prepare(self) -> None:
        with self._lock:
            if self._runtime is not None:
                return
            model_path = _local_model_path(self.model)
            runtime = native.VifuEmbeddedRuntime(_runtime_id("whisper"))
            runtime.register_whisper_provider(
                "provider",
                native.VifuWhisperProviderConfig(
                    model_path=str(model_path),
                    language=self.language,
                ),
            )
            runtime.register_agent(
                "transcriber",
                "Local Whisper",
                "provider",
                ["transcription"],
                "{}",
            )
            runtime.register_endpoint(
                "transcribe",
                "transcriber",
                "transcription",
                round(self.timeout * 1_000),
            )
            self._model_path = model_path
            self._runtime = runtime

    def transcribe_wav(self, wav: bytes, *, language: str | None = None) -> str:
        if not isinstance(wav, bytes) or not wav:
            raise ValueError("WAV audio must not be empty")
        selected_language = _language(language) or self.language
        result = self._invoke(
            native.VifuInvocationData.BINARY(wav),
            metadata={
                "binding": {
                    "language": selected_language,
                }
            }
            if selected_language
            else {},
        )
        text = result.get("text")
        if not isinstance(text, str):
            raise LocalProviderError("Local Whisper returned no transcription text")
        return text.strip()

    def _invoke(
        self,
        data: native.VifuInvocationData,
        *,
        metadata: Mapping[str, Any],
    ) -> dict[str, Any]:
        with self._lock:
            self.prepare()
            assert self._runtime is not None
            return _invoke(
                self._runtime,
                "transcribe",
                data,
                metadata=metadata,
                timeout=self.timeout,
            )

    def close(self) -> None:
        with self._lock:
            self._runtime = None
            self._model_path = None


class LocalLlama:
    """Resident llama.cpp chat Provider backed by the Vifu Runtime."""

    provider = "vifu-local-llama"
    supports_safe_accelerator_shutdown = True

    def __init__(
        self,
        *,
        model: str | Path,
        context_size: int = 4_096,
        default_max_tokens: int = 1_200,
        gpu_layers: int = 0,
        timeout: float = _DEFAULT_TIMEOUT_SECONDS,
    ) -> None:
        self.model = str(model)
        self.context_size = _positive_u32("context_size", context_size)
        self.default_max_tokens = _positive_u32(
            "default_max_tokens",
            default_max_tokens,
        )
        self.gpu_layers = _u32("gpu_layers", gpu_layers)
        self.timeout = _positive_timeout(timeout)
        self._model_path: Path | None = None
        self._runtime: native.VifuEmbeddedRuntime | None = None
        self._lock = threading.RLock()

    def prepare(self) -> None:
        with self._lock:
            if self._runtime is not None:
                return
            model_path = _local_model_path(self.model)
            runtime = native.VifuEmbeddedRuntime(_runtime_id("llama"))
            runtime.register_llama_provider(
                "provider",
                native.VifuLlamaProviderConfig(
                    model_path=str(model_path),
                    context_size=self.context_size,
                    gpu_layers=self.gpu_layers,
                    default_max_tokens=self.default_max_tokens,
                ),
            )
            runtime.register_agent(
                "model",
                "Local Llama",
                "provider",
                ["chat"],
                "{}",
            )
            runtime.register_endpoint(
                "chat",
                "model",
                "chat",
                round(self.timeout * 1_000),
            )
            self._model_path = model_path
            self._runtime = runtime

    def complete(
        self,
        request: Mapping[str, Any],
        *,
        session_id: str,
    ) -> dict[str, Any]:
        payload = dict(request)
        messages = payload.get("messages")
        if not isinstance(messages, list):
            raise ValueError("chat messages must be a list")
        responding_to_tool_result = bool(
            messages
            and isinstance(messages[-1], dict)
            and messages[-1].get("role") == "tool"
        )
        tool_name = None if responding_to_tool_result else _single_tool_name(payload)
        if tool_name is not None:
            function = payload["tools"][0]["function"]
            payload["response_format"] = {
                "type": "json_schema",
                "json_schema": {
                    "name": tool_name,
                    "strict": True,
                    "schema": function["parameters"],
                },
            }
        for key in ("parallel_tool_calls", "stream", "tool_choice", "tools", "user"):
            payload.pop(key, None)
        payload["messages"] = _local_chat_messages(messages)

        with self._lock:
            self.prepare()
            assert self._runtime is not None
            response = _invoke(
                self._runtime,
                "chat",
                native.VifuInvocationData.JSON(_encode_json(payload)),
                metadata={},
                session_id=session_id,
                timeout=self.timeout,
            )
        if tool_name is not None:
            return _tool_response(response, tool_name)
        return response

    def close(self) -> None:
        with self._lock:
            self._runtime = None
            self._model_path = None


def _invoke(
    runtime: native.VifuEmbeddedRuntime,
    endpoint: str,
    data: native.VifuInvocationData,
    *,
    metadata: Mapping[str, Any],
    timeout: float,
    session_id: str = "default",
) -> dict[str, Any]:
    handle = runtime.start_invoke(
        endpoint,
        session_id,
        data,
        _encode_json(dict(metadata)),
    )
    deadline = time.monotonic() + timeout
    while True:
        poll = runtime.take_invocation(handle)
        if poll.state == native.VifuInvocationState.COMPLETED:
            if poll.result is None or not poll.result.data.is_json():
                raise LocalProviderError("Vifu Provider returned an invalid result")
            value = json.loads(poll.result.data.json)
            if not isinstance(value, dict):
                raise LocalProviderError("Vifu Provider result must be an object")
            return value
        if poll.state in (
            native.VifuInvocationState.FAILED,
            native.VifuInvocationState.CANCELLED,
        ):
            raise LocalProviderError(
                poll.error or f"Vifu Provider ended with {poll.state.name.lower()}"
            )
        if time.monotonic() >= deadline:
            runtime.cancel_invocation(handle)
            raise TimeoutError(f"Vifu Provider exceeded {timeout:.1f} seconds")
        time.sleep(0.005)


def _local_model_path(value: str) -> Path:
    candidate = Path(value).expanduser()
    if not candidate.is_absolute() and candidate.parent == Path("."):
        candidate = Path.home() / ".vifu" / "models" / candidate
    candidate = candidate.resolve()
    if not candidate.is_file():
        model_name = Path(value).name or "model"
        raise FileNotFoundError(
            f"Vifu local model was not found: {model_name}. "
            "Place the model in ~/.vifu/models or pass an explicit model path."
        )
    return candidate


def _single_tool_name(payload: Mapping[str, Any]) -> str | None:
    tools = payload.get("tools")
    if tools is None:
        return None
    if not isinstance(tools, list) or len(tools) != 1:
        raise LocalProviderError("Local Llama supports exactly one Strands tool per turn")
    tool = tools[0]
    if not isinstance(tool, dict) or tool.get("type") != "function":
        raise LocalProviderError("Local Llama received an invalid Strands tool")
    function = tool.get("function")
    if not isinstance(function, dict):
        raise LocalProviderError("Local Llama received an invalid tool function")
    name = function.get("name")
    parameters = function.get("parameters")
    if not isinstance(name, str) or not name or not isinstance(parameters, dict):
        raise LocalProviderError("Local Llama tool name and parameters are required")
    return name


def _local_chat_messages(messages: list[Any]) -> list[dict[str, Any]]:
    converted: list[dict[str, Any]] = []
    for message in messages:
        if not isinstance(message, dict):
            raise ValueError("each chat message must be an object")
        role = message.get("role")
        if role == "tool":
            tool_call_id = message.get("tool_call_id")
            if not isinstance(tool_call_id, str) or not tool_call_id:
                raise ValueError("local tool result ID is required")
            content = _local_message_content(message.get("content"))
            converted.append(
                {
                    "role": "user",
                    "content": f"Tool result ({tool_call_id}):\n{content}",
                }
            )
            continue
        if role not in {"system", "user", "assistant"}:
            raise ValueError(f"local chat message role is unsupported: {role}")
        content = _local_message_content(message.get("content"))
        if role == "assistant":
            tool_calls = _local_tool_call_text(message.get("tool_calls"))
            content = "\n".join(part for part in (content, tool_calls) if part)
        converted.append(
            {
                "role": role,
                "content": content,
            }
        )
    return converted


def _local_message_content(value: Any) -> str:
    if value is None:
        return ""
    if not isinstance(value, str):
        raise ValueError("local chat message content must be text")
    return value


def _local_tool_call_text(value: Any) -> str:
    if value is None:
        return ""
    if not isinstance(value, list):
        raise ValueError("local chat tool calls must be a list")
    summaries: list[str] = []
    for tool_call in value:
        if not isinstance(tool_call, dict):
            raise ValueError("local chat tool call must be an object")
        tool_call_id = tool_call.get("id")
        function = tool_call.get("function")
        if (
            not isinstance(tool_call_id, str)
            or not tool_call_id
            or not isinstance(function, dict)
        ):
            raise ValueError("local chat tool call ID and function are required")
        name = function.get("name")
        arguments = function.get("arguments")
        if not isinstance(name, str) or not name or not isinstance(arguments, str):
            raise ValueError("local chat tool call name and arguments are required")
        summaries.append(f"Tool call {name} ({tool_call_id}):\n{arguments}")
    return "\n".join(summaries)


def _tool_response(response: dict[str, Any], tool_name: str) -> dict[str, Any]:
    choices = response.get("choices")
    message = (
        choices[0].get("message")
        if isinstance(choices, list) and choices and isinstance(choices[0], dict)
        else None
    )
    content = message.get("content") if isinstance(message, dict) else None
    if not isinstance(content, str):
        raise LocalProviderError("Local Llama returned no structured tool arguments")
    try:
        arguments = json.loads(content)
    except json.JSONDecodeError as error:
        raise LocalProviderError("Local Llama tool arguments are not valid JSON") from error
    if not isinstance(arguments, dict):
        raise LocalProviderError("Local Llama tool arguments must be an object")
    response = dict(response)
    response["choices"] = [
        {
            "index": 0,
            "message": {
                "role": "assistant",
                "content": None,
                "tool_calls": [
                    {
                        "id": f"call_{uuid.uuid4().hex}",
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": _encode_json(arguments),
                        },
                    }
                ],
            },
            "finish_reason": "tool_calls",
        }
    ]
    return response


def _language(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = value.strip().split("-", 1)[0].lower()
    return normalized or None


def _positive_timeout(value: float) -> float:
    timeout = float(value)
    if timeout <= 0:
        raise ValueError("timeout must be positive")
    return timeout


def _positive_u32(name: str, value: int) -> int:
    number = _u32(name, value)
    if number == 0:
        raise ValueError(f"{name} must be positive")
    return number


def _u32(name: str, value: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value < 2**32:
        raise ValueError(f"{name} must fit an unsigned 32-bit integer")
    return value


def _runtime_id(capability: str) -> str:
    return f"python-{capability}-{uuid.uuid4().hex}"


def _encode_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


__all__ = [
    "LocalLlama",
    "LocalProviderError",
    "LocalWhisper",
]
