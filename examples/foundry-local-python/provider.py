"""Adapt a Foundry Local native chat client to Vifu Runtime."""

from __future__ import annotations

from typing import Any, Iterable, Protocol

from vifu import AgentResponse, VifuRuntime


class FoundryChatClient(Protocol):
    def complete_streaming_chat(self, messages: list[dict[str, str]]) -> Iterable[Any]: ...


def register_foundry_agent(
    runtime: VifuRuntime,
    client: FoundryChatClient,
    *,
    model: str,
    endpoint: str = "foundry-chat",
) -> VifuRuntime:
    def invoke(request):
        prompt = request.input["prompt"]
        messages = [{"role": "user", "content": prompt}]
        chunks = iter(client.complete_streaming_chat(messages))
        parts: list[str] = []

        with request.trace.stage("first_token", metadata={"model": model}):
            for chunk in chunks:
                content = _chunk_content(chunk)
                if content:
                    parts.append(content)
                    request.trace.output_delta({"text": content})
                    break

        with request.trace.stage("decode", metadata={"model": model}):
            for chunk in chunks:
                content = _chunk_content(chunk)
                if content:
                    parts.append(content)
                    request.trace.output_delta({"text": content})

        return AgentResponse(
            output={"text": "".join(parts)},
            metadata={"model": model, "provider": "foundry-local"},
        )

    return runtime.agent(
        "foundry-local",
        invoke,
        endpoint=endpoint,
        metadata={"model": model, "framework": "foundry-local"},
    )


def _chunk_content(chunk: Any) -> str:
    choices = getattr(chunk, "choices", None)
    if not choices:
        return ""
    return getattr(choices[0].delta, "content", None) or ""
