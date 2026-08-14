"""Foundry Local integration for a Vifu Agent."""

from __future__ import annotations

import threading
from typing import Any, Iterable, Protocol

from ..runtime import AgentRequest, AgentResponse


class FoundryChatClient(Protocol):
    def complete_streaming_chat(
        self,
        messages: list[dict[str, str]],
    ) -> Iterable[Any]: ...


class FoundryLocal:
    """Loads a Foundry Local model and exposes it as a Vifu handler."""

    def __init__(
        self,
        model: str = "qwen2.5-0.5b",
        *,
        app_name: str = "vifu-foundry-local",
        client: FoundryChatClient | None = None,
        manager: Any | None = None,
    ):
        self.model = model
        self.app_name = app_name
        self.metadata = {"model": model, "framework": "foundry-local"}
        self._client = client
        self._manager = manager
        self._loaded_model: Any | None = None
        self._owns_model = False
        self._lock = threading.RLock()

    def prepare(self) -> None:
        """Downloads and loads the selected model once."""
        with self._lock:
            if self._client is not None:
                return
            manager = self._manager or _foundry_manager(self.app_name)
            print(f"Foundry Local: preparing {self.model}")
            manager.download_and_register_eps(
                progress_callback=lambda _name, _percent: None,
            )
            model = manager.catalog.get_model(self.model)
            model.download(lambda _progress: None)
            already_loaded = bool(getattr(model, "is_loaded", False))
            if not already_loaded:
                model.load()
            self._manager = manager
            self._loaded_model = model
            self._client = model.get_chat_client()
            self._owns_model = not already_loaded
            print(f"Foundry Local: {self.model} is ready")

    def __call__(self, request: AgentRequest) -> AgentResponse:
        with self._lock:
            self.prepare()
            if self._client is None:
                raise RuntimeError("Foundry Local did not create a chat client")
            prompt = _prompt(request.input)
            messages = _history(request.state)
            messages.append({"role": "user", "content": prompt})
            chunks = iter(self._client.complete_streaming_chat(list(messages)))
            parts: list[str] = []

            with request.trace.stage("first_token", metadata={"model": self.model}):
                for chunk in chunks:
                    content = _chunk_content(chunk)
                    if content:
                        parts.append(content)
                        request.trace.output_delta({"text": content})
                        break

            with request.trace.stage("decode", metadata={"model": self.model}):
                for chunk in chunks:
                    content = _chunk_content(chunk)
                    if content:
                        parts.append(content)
                        request.trace.output_delta({"text": content})

            text = "".join(parts)
            messages.append({"role": "assistant", "content": text})
            return AgentResponse(
                output={"text": text},
                metadata={"model": self.model, "provider": "foundry-local"},
                state={"messages": messages[-20:]},
            )

    def close(self) -> None:
        """Unloads a model that this integration loaded."""
        with self._lock:
            if self._owns_model and self._loaded_model is not None:
                self._loaded_model.unload()
                self._loaded_model = None
                self._client = None
                self._owns_model = False


def _foundry_manager(app_name: str) -> Any:
    try:
        from foundry_local_sdk import Configuration, FoundryLocalManager
    except ImportError as error:
        raise RuntimeError(
            'Install the Foundry integration with: python -m pip install "vifu[foundry]"'
        ) from error

    try:
        manager = FoundryLocalManager.instance
    except Exception:
        manager = None
    if manager is None:
        FoundryLocalManager.initialize(Configuration(app_name=app_name))
        manager = FoundryLocalManager.instance
    return manager


def _prompt(value: Any) -> str:
    if isinstance(value, str) and value.strip():
        return value.strip()
    if isinstance(value, dict):
        prompt = value.get("prompt")
        if isinstance(prompt, str) and prompt.strip():
            return prompt.strip()
    raise ValueError("Foundry Local input requires a non-empty prompt")


def _history(value: Any) -> list[dict[str, str]]:
    if not isinstance(value, dict):
        return []
    messages = value.get("messages")
    if not isinstance(messages, list):
        return []
    return [
        {"role": message["role"], "content": message["content"]}
        for message in messages[-18:]
        if isinstance(message, dict)
        and message.get("role") in {"user", "assistant"}
        and isinstance(message.get("content"), str)
    ]


def _chunk_content(chunk: Any) -> str:
    choices = getattr(chunk, "choices", None)
    if not choices:
        return ""
    return getattr(choices[0].delta, "content", None) or ""
