"""Ergonomic Python API over the generated Vifu UniFFI binding."""

from __future__ import annotations

import asyncio
import inspect
import json
import os
import re
import time
from contextlib import AbstractContextManager
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Awaitable, Callable, Mapping, TypeAlias

from . import vifu_mobile_ffi as native

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


@dataclass(frozen=True)
class AgentRequest:
    app_id: str
    endpoint: str
    session_id: str
    provider_id: str
    agent_id: str
    agent_name: str
    capabilities: tuple[str, ...]
    agent_metadata: JsonValue
    capability: str
    input: JsonValue
    metadata: JsonValue
    state: JsonValue
    state_revision: int
    trace: "AgentTrace"


@dataclass(frozen=True)
class AgentResponse:
    output: JsonValue
    metadata: JsonValue = None
    state: JsonValue | None = None


AgentHandler: TypeAlias = Callable[
    [AgentRequest],
    JsonValue | AgentResponse | Awaitable[JsonValue | AgentResponse],
]


class AgentStage(AbstractContextManager["AgentStage"]):
    def __init__(self, trace: "AgentTrace", name: str, metadata: JsonValue):
        self._trace = trace
        self._stage = _provider_stage(name)
        self._metadata = metadata
        self._started = time.monotonic()

    def __enter__(self) -> "AgentStage":
        self._trace._native.stage_started(self._stage, _encode_json(self._metadata))
        return self

    def __exit__(self, error_type, error, _traceback) -> bool:
        elapsed_ms = max(0, round((time.monotonic() - self._started) * 1_000))
        if error is None:
            self._trace._native.stage_completed(
                self._stage,
                elapsed_ms,
                _encode_json(self._metadata),
            )
        else:
            self._trace._native.stage_failed(
                self._stage,
                elapsed_ms,
                str(error),
                _encode_json(self._metadata),
            )
        return False


class AgentTrace:
    """Reports provider progress and stage timing to Vifu traces."""

    def __init__(self, invocation: native.VifuProviderInvocation):
        self._native = invocation

    def activity(self) -> None:
        self._native.activity()

    def stage(self, name: str, *, metadata: JsonValue = None) -> AgentStage:
        return AgentStage(self, name, metadata if metadata is not None else {})

    def output_delta(self, value: JsonValue) -> None:
        self._native.output_delta(native.VifuInvocationData.JSON(_encode_json(value)))

    @property
    def cancelled(self) -> bool:
        return self._native.is_cancelled()


@dataclass(frozen=True)
class Invocation:
    invocation_id: str
    app_id: str
    endpoint: str
    session_id: str
    agent_id: str
    provider_id: str
    capability: str
    output: JsonValue
    metadata: JsonValue
    state: JsonValue
    state_revision: int
    trace: list[dict[str, Any]]


class _PythonProvider:
    def __init__(self, handler: AgentHandler):
        self._handler = handler

    def invoke(
        self,
        request: native.VifuProviderRequest,
        invocation: native.VifuProviderInvocation,
    ) -> native.VifuProviderResponse:
        try:
            value = self._handler(_agent_request(request, invocation))
            if inspect.isawaitable(value):
                value = asyncio.run(value)
            response = value if isinstance(value, AgentResponse) else AgentResponse(output=value)
            return native.VifuProviderResponse(
                data=native.VifuInvocationData.JSON(_encode_json(response.output)),
                metadata_json=_encode_json(
                    response.metadata if response.metadata is not None else {}
                ),
                state_json=None if response.state is None else _encode_json(response.state),
            )
        except native.VifuRuntimeError:
            raise
        except Exception as error:
            raise native.VifuRuntimeError.Runtime(
                f"{type(error).__name__}: {error}"
            ) from error


class VifuRuntime:
    """One persistent Vifu application runtime embedded in the Python process."""

    def __init__(self, app_id: str, *, data_dir: str | os.PathLike[str] | None = None):
        if not re.fullmatch(r"[A-Za-z0-9_.:-]+", app_id):
            raise ValueError("app_id must contain only letters, numbers, '.', '_', ':', or '-'")
        root = Path(data_dir) if data_dir is not None else Path.home() / ".vifu" / "sdk" / "python" / app_id
        root.mkdir(parents=True, exist_ok=True)
        self.app_id = app_id
        self.data_dir = root
        self.database_path = root / "runtime.sqlite"
        self._native: native.VifuEmbeddedRuntime | None = native.VifuEmbeddedRuntime.open(
            app_id,
            str(self.database_path),
        )
        self._providers: list[_PythonProvider] = []

    def agent(
        self,
        agent_id: str,
        handler: AgentHandler,
        *,
        name: str | None = None,
        endpoint: str | None = None,
        provider_id: str | None = None,
        capability: str = "chat",
        timeout_ms: int = 30_000,
        metadata: JsonValue = None,
    ) -> VifuRuntime:
        provider_id = provider_id or f"{agent_id}-provider"
        endpoint = endpoint or agent_id
        provider = _PythonProvider(handler)
        runtime = self._native_runtime()
        runtime.register_streaming_provider(provider_id, "python", provider)
        runtime.register_agent(
            agent_id,
            name or agent_id,
            provider_id,
            [capability],
            _encode_json(metadata if metadata is not None else {}),
        )
        runtime.register_endpoint(endpoint, agent_id, capability, timeout_ms)
        self._providers.append(provider)
        return self

    def invoke(
        self,
        endpoint: str,
        input: JsonValue,
        *,
        session_id: str = "default",
        metadata: JsonValue = None,
        timeout: float = 35.0,
    ) -> Invocation:
        runtime = self._native_runtime()
        handle = runtime.start_invoke(
            endpoint,
            session_id,
            native.VifuInvocationData.JSON(_encode_json(input)),
            _encode_json(metadata if metadata is not None else {}),
        )
        deadline = time.monotonic() + timeout
        while True:
            poll = runtime.take_invocation(handle)
            if poll.state == native.VifuInvocationState.COMPLETED:
                if poll.result is None:
                    raise RuntimeError("Vifu completed the invocation without a result")
                return _invocation(poll.result)
            if poll.state in (native.VifuInvocationState.FAILED, native.VifuInvocationState.CANCELLED):
                raise RuntimeError(poll.error or f"Vifu invocation ended with {poll.state.name.lower()}")
            if time.monotonic() >= deadline:
                runtime.cancel_invocation(handle)
                raise TimeoutError(f"Vifu invocation exceeded {timeout:.1f} seconds")
            time.sleep(0.005)

    def pending_traces(self, limit: int = 100) -> list[dict[str, Any]]:
        return json.loads(self._native_runtime().pending_runtime_traces(limit))

    def acknowledge_traces(self, trace_ids: list[str]) -> None:
        self._native_runtime().acknowledge_runtime_traces(trace_ids)

    def export_snapshot(self) -> bytes:
        return self._native_runtime().export_snapshot()

    def restore_snapshot(self, snapshot: bytes) -> None:
        self._native_runtime().restore_snapshot(snapshot)

    def close(self) -> None:
        """Releases native providers and the Runtime database handle."""
        self._providers.clear()
        self._native = None

    def _native_runtime(self) -> native.VifuEmbeddedRuntime:
        runtime = self._native
        if runtime is None:
            raise RuntimeError("Vifu Runtime is closed")
        return runtime

    def connect(
        self,
        pairing_code: str | None = None,
        *,
        name: str | None = None,
        capture_trace_content: bool = False,
    ) -> "VifuGateway":
        from .gateway import VifuGateway

        return VifuGateway.connect(
            runtime=self,
            pairing_code=pairing_code,
            name=name,
            capture_trace_content=capture_trace_content,
        )

    def connect_local(
        self,
        *,
        server_url: str = "http://127.0.0.1:6790",
        name: str | None = None,
        capture_trace_content: bool = False,
        app_id: str | None = None,
    ) -> "VifuGateway":
        """Connects this Runtime to an App on a loopback Vifu Server."""
        from .gateway import VifuGateway

        return VifuGateway.connect(
            runtime=self,
            pairing_code=None,
            name=name,
            capture_trace_content=capture_trace_content,
            local_server_url=server_url,
            local_app_id=app_id,
        )


def _agent_request(
    request: native.VifuProviderRequest,
    invocation: native.VifuProviderInvocation,
) -> AgentRequest:
    return AgentRequest(
        app_id=request.project_id,
        endpoint=request.endpoint,
        session_id=request.session_id,
        provider_id=request.provider_id,
        agent_id=request.agent_id,
        agent_name=request.agent_name,
        capabilities=tuple(request.agent_capabilities),
        agent_metadata=json.loads(request.agent_metadata_json),
        capability=request.capability,
        input=_decode_data(request.data),
        metadata=json.loads(request.metadata_json),
        state=json.loads(request.state_json),
        state_revision=request.state_revision,
        trace=AgentTrace(invocation),
    )


def _invocation(result: native.VifuInvocationResult) -> Invocation:
    return Invocation(
        invocation_id=result.invocation_id,
        app_id=result.project_id,
        endpoint=result.endpoint,
        session_id=result.session_id,
        agent_id=result.agent_id,
        provider_id=result.provider_id,
        capability=result.capability,
        output=_decode_data(result.data),
        metadata=json.loads(result.metadata_json),
        state=json.loads(result.state_json),
        state_revision=result.state_revision,
        trace=json.loads(result.trace_json),
    )


def _decode_data(data: native.VifuInvocationData) -> JsonValue:
    if data.is_json():
        return json.loads(data.json)
    return {"_vifuBinary": True, "bytes": list(data.bytes)}


def _encode_json(value: JsonValue) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def _provider_stage(name: str) -> native.VifuProviderStage:
    normalized = name.strip().lower().replace("-", "_")
    stages = {
        "queue": native.VifuProviderStage.QUEUE,
        "load": native.VifuProviderStage.LOAD,
        "tokenize": native.VifuProviderStage.TOKENIZE,
        "prefill": native.VifuProviderStage.PREFILL,
        "first_token": native.VifuProviderStage.FIRST_TOKEN,
        "decode": native.VifuProviderStage.DECODE,
        "validate": native.VifuProviderStage.VALIDATE,
    }
    try:
        return stages[normalized]
    except KeyError as error:
        raise ValueError(
            "stage must be queue, load, tokenize, prefill, first_token, decode, or validate"
        ) from error
