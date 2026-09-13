"""Code-configured Providers that run inside a Python Vifu App."""

from __future__ import annotations

import json
import re
import threading
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from types import MappingProxyType
from typing import Any, Mapping
from urllib.parse import urlsplit

from . import vifu_mobile_ffi as native

_DEFAULT_TIMEOUT_SECONDS = 70.0
_MAX_HTTP_RESPONSE_BYTES = 2 * 1024 * 1024
_IDENTIFIER = re.compile(r"^[A-Za-z0-9_.:-]+$")
_SENSITIVE_SETTING_KEYS = (
    "authorization",
    "apikey",
    "accesstoken",
    "refreshtoken",
    "token",
    "secret",
    "password",
    "credential",
    "cookie",
    "session",
    "sessionid",
)


class LocalProviderError(RuntimeError):
    """An in-process Vifu Provider could not start or complete an invocation."""


class OpenAICompatible:
    """Code-configured OpenAI-compatible chat Provider for one Vifu App."""

    provider = "vifu-openai-compatible"
    vifu_name = "OpenAI-compatible"
    vifu_provider_type = "openai-compatible"
    vifu_capabilities = ("chat",)

    def __init__(
        self,
        *,
        url: str,
        model: str,
        api_key: str | None = None,
        timeout: float = _DEFAULT_TIMEOUT_SECONDS,
    ) -> None:
        self.url = _provider_url(url)
        self.model = model.strip()
        if not self.model or len(self.model) > 256:
            raise ValueError("OpenAI-compatible Provider model is required")
        self.api_key = api_key.strip() if api_key and api_key.strip() else None
        self.timeout = _positive_timeout(timeout)

    @property
    def vifu_settings(self) -> dict[str, Any]:
        return {"url": self.url, "model": self.model}

    def complete(
        self,
        request: Mapping[str, Any],
        *,
        session_id: str,
    ) -> dict[str, Any]:
        payload = dict(request)
        if not isinstance(payload.get("messages"), list):
            raise ValueError("chat messages must be a list")
        payload["model"] = self.model
        payload["stream"] = False
        if session_id.strip():
            payload.setdefault("user", session_id.strip())
        headers = {
            "Content-Type": "application/json",
            "User-Agent": "Vifu-Python-SDK",
        }
        if self.api_key is not None:
            headers["Authorization"] = f"Bearer {self.api_key}"
        http_request = urllib.request.Request(
            self.url,
            data=_encode_json(payload).encode("utf-8"),
            method="POST",
            headers=headers,
        )
        try:
            with urllib.request.urlopen(http_request, timeout=self.timeout) as response:
                body = response.read(_MAX_HTTP_RESPONSE_BYTES + 1)
        except urllib.error.HTTPError as error:
            raise LocalProviderError(
                f"OpenAI-compatible Provider returned HTTP {error.code}"
            ) from error
        except (OSError, TimeoutError, urllib.error.URLError) as error:
            raise LocalProviderError("OpenAI-compatible Provider request failed") from error
        if len(body) > _MAX_HTTP_RESPONSE_BYTES:
            raise LocalProviderError("OpenAI-compatible Provider response exceeds 2 MiB")
        try:
            value = json.loads(body)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise LocalProviderError(
                "OpenAI-compatible Provider response is not valid JSON"
            ) from error
        if not isinstance(value, dict):
            raise LocalProviderError("OpenAI-compatible Provider response must be an object")
        return value


@dataclass(frozen=True)
class AppProvider:
    """One Provider in a Vifu App's private Provider list."""

    key: str
    name: str
    provider_type: str
    capabilities: tuple[str, ...]
    settings: Mapping[str, Any] = field(repr=False)
    resources: Mapping[str, str] = field(repr=False)
    _implementation: Any = field(repr=False, compare=False)

    def __getattr__(self, name: str) -> Any:
        implementation = object.__getattribute__(self, "_implementation")
        return getattr(implementation, name)

    def descriptor(self) -> dict[str, Any]:
        """Return the secret-free Gateway description for this Provider."""
        return {
            "id": self.key,
            "name": self.name,
            "type": "vifu-runtime",
            "localProviderType": self.provider_type,
            "capabilities": list(self.capabilities),
            "settings": _public_mapping(self.settings),
            "resources": _public_mapping(self.resources),
            "source": "private",
        }

    @property
    def implementation(self) -> Any:
        return self._implementation


def app_provider(
    key: str,
    implementation: Any,
    *,
    name: str | None = None,
    provider_type: str | None = None,
    capabilities: tuple[str, ...] | list[str] | None = None,
    settings: Mapping[str, Any] | None = None,
    resources: Mapping[str, str] | None = None,
) -> AppProvider:
    """Normalize one code-configured App-private Provider."""
    key = _identifier("provider key", key)
    if implementation is None:
        raise ValueError("an App Provider requires an implementation")
    inferred_type = (
        provider_type
        or getattr(implementation, "vifu_provider_type", None)
    )
    if not isinstance(inferred_type, str):
        raise ValueError("provider_type is required for this App Provider")
    normalized_type = _identifier("provider type", inferred_type)
    inferred_capabilities = capabilities or getattr(
        implementation,
        "vifu_capabilities",
        None,
    )
    if not inferred_capabilities:
        raise ValueError("App Provider capabilities must not be empty")
    normalized_capabilities = tuple(
        dict.fromkeys(
            _identifier("provider capability", str(capability))
            for capability in inferred_capabilities
        )
    )
    inferred_settings = settings
    if inferred_settings is None:
        inferred_settings = getattr(implementation, "vifu_settings", {})
    inferred_resources = resources
    if inferred_resources is None:
        inferred_resources = getattr(implementation, "vifu_resources", {})
    if not isinstance(inferred_settings, Mapping):
        raise TypeError("provider settings must be a mapping")
    if not isinstance(inferred_resources, Mapping) or not all(
        isinstance(key, str) and isinstance(value, str)
        for key, value in inferred_resources.items()
    ):
        raise TypeError("provider resources must map strings to strings")
    display_name = (name or getattr(implementation, "vifu_name", None) or key).strip()
    if not display_name:
        raise ValueError("provider name must not be empty")
    return AppProvider(
        key=key,
        name=display_name,
        provider_type=normalized_type,
        capabilities=normalized_capabilities,
        settings=MappingProxyType(dict(inferred_settings)),
        resources=MappingProxyType(dict(inferred_resources)),
        _implementation=implementation,
    )


class LocalWhisper:
    """Resident whisper.cpp transcription Provider backed by the Vifu Runtime."""

    provider = "vifu-local-whisper"
    vifu_name = "Local Whisper"
    vifu_provider_type = "local-whisper"
    vifu_capabilities = ("transcription",)

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

    @property
    def vifu_settings(self) -> dict[str, Any]:
        return {
            "model": Path(self.model).name,
            **({"language": self.language} if self.language else {}),
        }

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
    vifu_name = "Local Llama"
    vifu_provider_type = "llama"
    vifu_capabilities = ("chat",)
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

    @property
    def vifu_settings(self) -> dict[str, Any]:
        return {
            "model": Path(self.model).name,
            "contextSize": self.context_size,
            "maxTokens": self.default_max_tokens,
            "gpuLayers": self.gpu_layers,
        }

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


def _identifier(label: str, value: str) -> str:
    normalized = value.strip()
    if not normalized or len(normalized) > 128 or _IDENTIFIER.fullmatch(normalized) is None:
        raise ValueError(
            f"{label} must contain only letters, numbers, '.', '_', ':', or '-'"
        )
    return normalized


def _provider_url(value: str) -> str:
    normalized = value.strip()
    parsed = urlsplit(normalized)
    loopback = parsed.hostname in {"127.0.0.1", "localhost", "::1"}
    allowed_schemes = {"http", "https"} if loopback else {"https"}
    if (
        parsed.scheme not in allowed_schemes
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise ValueError(
            "OpenAI-compatible Provider URL must use HTTPS or loopback HTTP "
            "without credentials, query, or fragment"
        )
    return normalized


def _public_mapping(value: Mapping[str, Any]) -> dict[str, Any]:
    public: dict[str, Any] = {}
    for key, item in value.items():
        normalized = "".join(character for character in key.lower() if character.isalnum())
        if any(
            normalized == candidate or normalized.endswith(candidate)
            for candidate in _SENSITIVE_SETTING_KEYS
        ):
            continue
        if isinstance(item, Mapping):
            public[key] = _public_mapping(item)
        elif isinstance(item, (str, int, float, bool)) or item is None:
            public[key] = item
    return public


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
    "AppProvider",
    "LocalLlama",
    "LocalProviderError",
    "LocalWhisper",
    "OpenAICompatible",
]
