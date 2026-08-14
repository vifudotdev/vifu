"""Transparent tracing for Foundry Local streaming inference."""

from __future__ import annotations

from collections.abc import Callable, Iterable, Iterator
from typing import Any, TypeVar

from ..runtime import AgentRequest


Chunk = TypeVar("Chunk")


def trace_foundry_stream(
    request: AgentRequest,
    chunks: Iterable[Chunk],
    *,
    model: str,
    content: Callable[[Chunk], str] | None = None,
) -> Iterator[Chunk]:
    """Yields the original Foundry stream while recording inference stages.

    Foundry Local still owns model setup and inference. The application still
    owns messages, output assembly, and business behavior.
    """

    read_content = content or foundry_chunk_text
    stream = iter(chunks)
    with request.trace.stage("first_token", metadata={"model": model}):
        try:
            first = next(stream)
        except StopIteration:
            return
        _report_delta(request, read_content(first))
        yield first

    with request.trace.stage("decode", metadata={"model": model}):
        for chunk in stream:
            _report_delta(request, read_content(chunk))
            yield chunk


def _report_delta(request: AgentRequest, value: str) -> None:
    if value:
        request.trace.output_delta({"text": value})


def foundry_chunk_text(chunk: Any) -> str:
    """Returns text from a Foundry chat chunk, or empty text for control chunks."""
    choices = getattr(chunk, "choices", None)
    if not choices:
        return ""
    delta = getattr(choices[0], "delta", None)
    return getattr(delta, "content", None) or ""
