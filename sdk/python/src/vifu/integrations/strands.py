"""Strands Model adapter backed by a Vifu Agent Profile."""

from __future__ import annotations

import asyncio
import json
import os
import re
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, AsyncGenerator, Iterable
from urllib.parse import quote, urlparse

from .._version import __version__
from ..app import Vifu
from ..runtime import AgentRequest

_MAX_CREDENTIAL_BYTES = 64 * 1024
_MAX_RESPONSE_BYTES = 2 * 1024 * 1024
_PROFILE_PATTERN = re.compile(r"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$")


@dataclass(frozen=True)
class _ModelTarget:
    url: str
    profile: str
    invocation_id: str
    token: str | None = field(default=None, repr=False)
    expires_at: datetime | None = None

    @classmethod
    def for_app(
        cls,
        app: Vifu,
        request: AgentRequest,
        profile: str,
    ) -> "_ModelTarget":
        profile = _profile(profile)
        credential_file = os.environ.get("VIFU_PROVIDER_CREDENTIAL_FILE", "").strip()
        if credential_file:
            return cls.from_file(Path(credential_file), profile)
        return cls(
            url=f"{app.server_url.rstrip('/')}/{quote(app.slug)}/v1/chat/completions",
            profile=profile,
            invocation_id=request.session_id,
        )

    @classmethod
    def for_openai_compatible(
        cls,
        request: AgentRequest,
        *,
        url: str,
        model: str,
        api_key: str | None = None,
    ) -> "_ModelTarget":
        url = url.strip()
        model = model.strip()
        if not _secure_url(url):
            raise ValueError("OpenAI-compatible Provider URL must use HTTPS or loopback")
        if not model or len(model) > 256:
            raise ValueError("OpenAI-compatible Provider model is required")
        invocation_id = request.session_id.strip()
        if not invocation_id:
            raise ValueError("Agent request session ID is required")
        return cls(
            url=url,
            profile=model,
            invocation_id=invocation_id,
            token=api_key.strip() if api_key and api_key.strip() else None,
        )

    @classmethod
    def from_file(cls, path: Path, expected_profile: str) -> "_ModelTarget":
        try:
            data = path.read_bytes()
        except OSError as error:
            raise RuntimeError("Vifu Provider credential is unavailable") from error
        if len(data) > _MAX_CREDENTIAL_BYTES:
            raise ValueError("Vifu Provider credential exceeds 64 KiB")
        try:
            value = json.loads(data)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError("Vifu Provider credential is invalid") from error
        if not isinstance(value, dict):
            raise ValueError("Vifu Provider credential must be an object")
        url = value.get("url")
        token = value.get("token")
        invocation_id = value.get("invocationId")
        expires_at = value.get("expiresAt")
        if not isinstance(url, str) or not _secure_url(
            url,
            allow_loopback_http=False,
        ):
            raise ValueError("Vifu Provider URL must use HTTPS")
        if not isinstance(token, str) or not token:
            raise ValueError("Vifu Provider credential token is required")
        if not isinstance(invocation_id, str) or not invocation_id:
            raise ValueError("Vifu Provider invocation identity is required")
        if not isinstance(expires_at, str):
            raise ValueError("Vifu Provider credential expiry is required")
        expiry = _timestamp(expires_at)
        if expiry <= datetime.now(timezone.utc):
            raise ValueError("Vifu Provider credential has expired")
        return cls(
            url=url,
            profile=expected_profile,
            invocation_id=invocation_id,
            token=token,
            expires_at=expiry,
        )


def strands_model(
    app: Vifu,
    request: AgentRequest,
    *,
    profile: str,
    temperature: float = 0.2,
    max_tokens: int = 1_200,
) -> Any:
    """Returns a Strands Model routed through one Vifu Agent Profile."""

    target = _ModelTarget.for_app(app, request, profile)
    return _strands_model_for_target(
        target,
        temperature=temperature,
        max_tokens=max_tokens,
    )


def openai_compatible_model(
    request: AgentRequest,
    *,
    url: str,
    model: str,
    api_key: str | None = None,
    temperature: float = 0.2,
    max_tokens: int = 1_200,
) -> Any:
    """Returns a Strands Model backed by a code-configured compatible API."""

    target = _ModelTarget.for_openai_compatible(
        request,
        url=url,
        model=model,
        api_key=api_key,
    )
    return _strands_model_for_target(
        target,
        temperature=temperature,
        max_tokens=max_tokens,
    )


def local_provider_model(
    request: AgentRequest,
    *,
    provider: Any,
    temperature: float = 0.2,
    max_tokens: int = 1_200,
) -> Any:
    """Returns a Strands Model backed by a code-configured Vifu Provider."""

    complete = getattr(provider, "complete", None)
    if not callable(complete):
        raise TypeError("local Provider must define complete(request, session_id=...)")
    prepare = getattr(provider, "prepare", None)
    if callable(prepare):
        prepare()
    profile = str(getattr(provider, "model", "local-provider")).strip()
    if not profile:
        raise ValueError("local Provider model must not be empty")
    invocation_id = request.session_id.strip()
    if not invocation_id:
        raise ValueError("Agent request session ID is required")
    return _strands_model(
        profile=profile,
        invocation_id=invocation_id,
        complete=lambda payload: complete(payload, session_id=invocation_id),
        temperature=temperature,
        max_tokens=max_tokens,
    )


def _strands_model_for_target(
    target: _ModelTarget,
    *,
    temperature: float,
    max_tokens: int,
) -> Any:
    return _strands_model(
        profile=target.profile,
        invocation_id=target.invocation_id,
        complete=lambda payload: _post_json(target, payload),
        temperature=temperature,
        max_tokens=max_tokens,
    )


def _strands_model(
    *,
    profile: str,
    invocation_id: str,
    complete: Any,
    temperature: float,
    max_tokens: int,
) -> Any:
    """Builds the common Strands adapter after resolving a Provider target."""

    from strands.models import Model

    class VifuProfileModel(Model):
        def __init__(self) -> None:
            self._temperature = float(temperature)
            self._max_tokens = int(max_tokens)

        def get_config(self) -> dict[str, Any]:
            return {
                "model_id": profile,
                "temperature": self._temperature,
                "max_tokens": self._max_tokens,
            }

        def update_config(self, **model_config: Any) -> None:
            unsupported = set(model_config) - {"temperature", "max_tokens"}
            if unsupported:
                raise ValueError("a Vifu model cannot change its Agent Profile")
            if "temperature" in model_config:
                self._temperature = float(model_config["temperature"])
            if "max_tokens" in model_config:
                self._max_tokens = int(model_config["max_tokens"])

        async def stream(
            self,
            messages: list[dict[str, Any]],
            tool_specs: list[dict[str, Any]] | None = None,
            system_prompt: Any = None,
            *,
            tool_choice: Any = None,
            system_prompt_content: Any = None,
            **_kwargs: Any,
        ) -> AsyncGenerator[dict[str, Any], None]:
            payload = _openai_request(
                target=_ModelTarget(
                    url="http://127.0.0.1/unused",
                    profile=profile,
                    invocation_id=invocation_id,
                ),
                system_prompt=system_prompt,
                messages=messages,
                tool_specs=tool_specs,
                temperature=self._temperature,
                max_tokens=self._max_tokens,
                tool_choice=tool_choice,
                system_prompt_content=system_prompt_content,
            )
            started = time.monotonic()
            response = await asyncio.to_thread(complete, payload)
            latency_ms = round((time.monotonic() - started) * 1_000)
            for event in _strands_events(response, latency_ms):
                yield event

        async def structured_output(
            self,
            *_args: Any,
            **_kwargs: Any,
        ) -> AsyncGenerator[dict[str, Any], None]:
            if False:
                yield {}
            raise NotImplementedError("Vifu Agent Profile structured output is not implemented")

    return VifuProfileModel()


def _openai_request(
    *,
    target: _ModelTarget,
    system_prompt: Any,
    messages: Iterable[dict[str, Any]],
    tool_specs: Iterable[dict[str, Any]] | None,
    temperature: float,
    max_tokens: int,
    tool_choice: Any = None,
    system_prompt_content: Any = None,
) -> dict[str, Any]:
    openai_messages: list[dict[str, Any]] = []
    prompt_text = _content_text(
        system_prompt if system_prompt is not None else system_prompt_content
    )
    if prompt_text:
        openai_messages.append({"role": "system", "content": prompt_text})

    for message in messages:
        if not isinstance(message, dict):
            raise ValueError("Strands message must be an object")
        role = message.get("role")
        content = message.get("content")
        if role not in {"user", "assistant"} or not isinstance(content, list):
            raise ValueError("Strands message role or content is invalid")
        text_parts: list[str] = []
        tool_calls: list[dict[str, Any]] = []
        tool_results: list[dict[str, Any]] = []
        for block in content:
            if not isinstance(block, dict):
                raise ValueError("Strands content block must be an object")
            if isinstance(block.get("text"), str):
                text_parts.append(block["text"])
                continue
            tool_use = block.get("toolUse")
            if isinstance(tool_use, dict):
                tool_calls.append(
                    {
                        "id": _required_text(tool_use.get("toolUseId"), "toolUseId"),
                        "type": "function",
                        "function": {
                            "name": _required_text(tool_use.get("name"), "tool name"),
                            "arguments": json.dumps(
                                tool_use.get("input", {}),
                                ensure_ascii=False,
                                allow_nan=False,
                                separators=(",", ":"),
                            ),
                        },
                    }
                )
                continue
            tool_result = block.get("toolResult")
            if isinstance(tool_result, dict):
                tool_results.append(tool_result)
                continue
            raise ValueError("the selected Vifu Provider does not support this content block")

        text = "\n".join(part for part in text_parts if part)
        if tool_results:
            if role != "user" or tool_calls:
                raise ValueError("Strands tool results must be user content")
            if text:
                openai_messages.append({"role": "user", "content": text})
            for result in tool_results:
                openai_messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": _required_text(
                            result.get("toolUseId"), "tool result ID"
                        ),
                        "content": _tool_result_text(result.get("content")),
                    }
                )
            continue

        converted: dict[str, Any] = {"role": role, "content": text or None}
        if tool_calls:
            if role != "assistant":
                raise ValueError("Strands tool calls must be assistant content")
            converted["tool_calls"] = tool_calls
        elif converted["content"] is None:
            converted["content"] = ""
        openai_messages.append(converted)

    payload: dict[str, Any] = {
        "model": target.profile,
        "messages": openai_messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "stream": False,
        "user": target.invocation_id,
    }
    tools = [_openai_tool(spec) for spec in tool_specs or []]
    if tools:
        payload["tools"] = tools
        payload["parallel_tool_calls"] = False
        selected = _openai_tool_choice(tool_choice)
        if selected is not None:
            payload["tool_choice"] = selected
    return payload


def _post_json(target: _ModelTarget, payload: dict[str, Any]) -> dict[str, Any]:
    if target.expires_at is not None and target.expires_at <= datetime.now(timezone.utc):
        raise RuntimeError("Vifu Provider credential has expired")
    headers = {
        "Content-Type": "application/json",
        "User-Agent": f"Vifu-Python-SDK/{__version__}",
    }
    if target.token:
        headers["Authorization"] = f"Bearer {target.token}"
        headers["X-Vifu-Invocation-Id"] = target.invocation_id
    request = urllib.request.Request(
        target.url,
        data=json.dumps(
            payload,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
        ).encode("utf-8"),
        method="POST",
        headers=headers,
    )
    try:
        with urllib.request.urlopen(request, timeout=65) as response:
            body = response.read(_MAX_RESPONSE_BYTES + 1)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"Vifu Provider request failed with HTTP {error.code}") from error
    except (OSError, TimeoutError, urllib.error.URLError) as error:
        raise RuntimeError("Vifu Provider request failed") from error
    if len(body) > _MAX_RESPONSE_BYTES:
        raise RuntimeError("Vifu Provider response exceeds 2 MiB")
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError("Vifu Provider response is not valid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError("Vifu Provider response must be an object")
    return value


def _strands_events(response: dict[str, Any], latency_ms: int) -> list[dict[str, Any]]:
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
        raise RuntimeError("Vifu Provider response has no choice")
    choice = choices[0]
    message = choice.get("message")
    if not isinstance(message, dict):
        raise RuntimeError("Vifu Provider response has no assistant message")
    events: list[dict[str, Any]] = [{"messageStart": {"role": "assistant"}}]
    content = message.get("content")
    if isinstance(content, str) and content:
        events.extend(
            [
                {"contentBlockStart": {"start": {}}},
                {"contentBlockDelta": {"delta": {"text": content}}},
                {"contentBlockStop": {}},
            ]
        )
    tool_calls = message.get("tool_calls")
    if tool_calls is not None and not isinstance(tool_calls, list):
        raise RuntimeError("Vifu Provider tool calls are invalid")
    for tool_call in tool_calls or []:
        if not isinstance(tool_call, dict) or not isinstance(tool_call.get("function"), dict):
            raise RuntimeError("Vifu Provider tool call is invalid")
        function = tool_call["function"]
        events.extend(
            [
                {
                    "contentBlockStart": {
                        "start": {
                            "toolUse": {
                                "toolUseId": _required_text(tool_call.get("id"), "tool call ID"),
                                "name": _required_text(function.get("name"), "tool call name"),
                            }
                        }
                    }
                },
                {
                    "contentBlockDelta": {
                        "delta": {
                            "toolUse": {
                                "input": _required_text(function.get("arguments"), "tool arguments")
                            }
                        }
                    }
                },
                {"contentBlockStop": {}},
            ]
        )
    events.append({"messageStop": {"stopReason": _stop_reason(choice.get("finish_reason"))}})
    usage = response.get("usage") if isinstance(response.get("usage"), dict) else {}
    input_tokens = _token_count(usage.get("prompt_tokens"))
    output_tokens = _token_count(usage.get("completion_tokens"))
    events.append(
        {
            "metadata": {
                "usage": {
                    "inputTokens": input_tokens,
                    "outputTokens": output_tokens,
                    "totalTokens": _token_count(
                        usage.get("total_tokens"), input_tokens + output_tokens
                    ),
                },
                "metrics": {"latencyMs": max(0, int(latency_ms))},
            }
        }
    )
    return events


def _openai_tool(spec: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(spec, dict):
        raise ValueError("Strands tool spec must be an object")
    wrapper = spec.get("inputSchema", {})
    schema = wrapper.get("json") if isinstance(wrapper, dict) else None
    if not isinstance(schema, dict):
        schema = {"type": "object", "properties": {}}
    return {
        "type": "function",
        "function": {
            "name": _required_text(spec.get("name"), "tool name"),
            "description": spec.get("description") if isinstance(spec.get("description"), str) else "",
            "parameters": schema,
        },
    }


def _openai_tool_choice(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, str) and value in {"auto", "none", "required"}:
        return value
    if not isinstance(value, dict):
        raise ValueError("Strands tool choice is invalid")
    if "auto" in value:
        return "auto"
    if "any" in value:
        return "required"
    selected = value.get("tool")
    if isinstance(selected, dict):
        return {
            "type": "function",
            "function": {"name": _required_text(selected.get("name"), "tool choice")},
        }
    raise ValueError("Strands tool choice is invalid")


def _tool_result_text(content: Any) -> str:
    if not isinstance(content, list):
        raise ValueError("Strands tool result content must be a list")
    parts: list[str] = []
    for block in content:
        if not isinstance(block, dict):
            raise ValueError("Strands tool result block must be an object")
        if isinstance(block.get("text"), str):
            parts.append(block["text"])
        elif "json" in block:
            parts.append(
                json.dumps(
                    block["json"],
                    ensure_ascii=False,
                    allow_nan=False,
                    separators=(",", ":"),
                )
            )
        else:
            raise ValueError("the selected Vifu Provider does not support this tool result")
    return "\n".join(parts)


def _content_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        parts: list[str] = []
        for block in value:
            if isinstance(block, dict) and isinstance(block.get("text"), str):
                parts.append(block["text"])
            else:
                raise ValueError("Vifu Provider supports text system prompts")
        return "\n".join(parts)
    raise ValueError("Vifu Provider system prompt is invalid")


def _profile(value: str) -> str:
    normalized = value.strip()
    if not _PROFILE_PATTERN.fullmatch(normalized):
        raise ValueError("Vifu Agent Profile slug is invalid")
    return normalized


def _required_text(value: Any, field_name: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field_name} is required")
    return value


def _token_count(value: Any, default: int = 0) -> int:
    return value if isinstance(value, int) and value >= 0 else default


def _stop_reason(value: Any) -> str:
    return {
        "tool_calls": "tool_use",
        "length": "max_tokens",
        "stop": "end_turn",
        "content_filter": "content_filtered",
    }.get(value, "end_turn")


def _timestamp(value: str) -> datetime:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("Vifu Provider credential expiry is invalid") from error
    if parsed.tzinfo is None:
        raise ValueError("Vifu Provider credential expiry must include a timezone")
    return parsed.astimezone(timezone.utc)


def _secure_url(value: str, *, allow_loopback_http: bool = True) -> bool:
    parsed = urlparse(value)
    if parsed.hostname is None or parsed.username is not None or parsed.password is not None:
        return False
    if parsed.scheme == "https":
        return True
    return (
        allow_loopback_http
        and parsed.scheme == "http"
        and parsed.hostname in {"127.0.0.1", "localhost", "::1"}
    )
