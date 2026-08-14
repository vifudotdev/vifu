"""High-level API for a Python Agent application."""

from __future__ import annotations

import hashlib
import re
import time
from pathlib import Path
from typing import Callable

from .gateway import DEFAULT_LOCAL_SERVER_URL, VifuGateway
from .runtime import AgentHandler, Invocation, JsonValue, VifuRuntime


class Vifu:
    """Runs Python Agents in Vifu and connects them to the local Server."""

    def __init__(
        self,
        name: str,
        *,
        data_dir: str | Path | None = None,
        server_url: str = DEFAULT_LOCAL_SERVER_URL,
        capture_trace_content: bool = False,
    ):
        self.name = _display_name(name)
        self.runtime = VifuRuntime(_runtime_id(self.name), data_dir=data_dir)
        self.server_url = server_url
        self.capture_trace_content = capture_trace_content
        self._gateway: VifuGateway | None = None

    def agent(
        self,
        agent_id: str,
        *,
        name: str | None = None,
        endpoint: str | None = None,
        provider_id: str | None = None,
        capability: str = "chat",
        timeout_ms: int = 30_000,
        metadata: JsonValue = None,
    ) -> Callable[[AgentHandler], AgentHandler]:
        """Registers a decorated Python function as an Agent."""

        def register(handler: AgentHandler) -> AgentHandler:
            self.runtime.agent(
                agent_id,
                handler,
                name=name,
                endpoint=endpoint,
                provider_id=provider_id,
                capability=capability,
                timeout_ms=timeout_ms,
                metadata=metadata,
            )
            return handler

        return register

    def invoke(
        self,
        endpoint: str,
        input: JsonValue,
        *,
        session_id: str = "default",
        metadata: JsonValue = None,
        timeout: float = 35.0,
    ) -> Invocation:
        """Invokes an Agent endpoint in this process."""
        return self.runtime.invoke(
            endpoint,
            input,
            session_id=session_id,
            metadata=metadata,
            timeout=timeout,
        )

    def connect(self, *, timeout: float = 20.0) -> VifuGateway:
        """Connects to the local Server and waits until the Gateway is ready."""
        if self._gateway is None:
            self._gateway = self.runtime.connect_local(
                server_url=self.server_url,
                name=f"Python: {self.name}",
                capture_trace_content=self.capture_trace_content,
            )
        try:
            self._gateway.wait_until_connected(timeout)
        except TimeoutError as error:
            self.close()
            raise ConnectionError(
                f"The local Vifu Server did not answer at {self.server_url}. "
                "Start Vifu, then run this application again."
            ) from error
        return self._gateway

    def run(self, *, connect_timeout: float = 20.0) -> None:
        """Connects to Vifu and serves Agent calls until the process stops."""
        self.connect(timeout=connect_timeout)
        try:
            while True:
                time.sleep(3_600)
        except KeyboardInterrupt:
            pass
        finally:
            self.close()

    def close(self) -> None:
        """Stops the local Gateway connection."""
        if self._gateway is None:
            return
        gateway = self._gateway
        self._gateway = None
        gateway.close()

    def __enter__(self) -> "Vifu":
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()


def _display_name(name: str) -> str:
    value = name.strip()
    if not value:
        raise ValueError("name must not be empty")
    return value


def _runtime_id(name: str) -> str:
    readable = re.sub(r"[^A-Za-z0-9]+", "-", name).strip("-").lower()[:40]
    digest = hashlib.sha256(name.encode("utf-8")).hexdigest()[:12]
    return f"python-{readable or 'agent'}-{digest}"
