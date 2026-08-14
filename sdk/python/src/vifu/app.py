"""High-level API for a Python Agent application."""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlparse

from .app_store import VifuAppRecord, VifuAppStore
from .gateway import DEFAULT_LOCAL_SERVER_URL, VifuGateway
from .runtime import AgentHandler, Invocation, JsonValue, VifuRuntime
from .server import VifuServer


class Vifu:
    """Runs Python Agents in Vifu and connects them to the local Server."""

    def __init__(
        self,
        name: str,
        *,
        data_dir: str | Path | None = None,
        workspace: str | Path | None = None,
        server_url: str = DEFAULT_LOCAL_SERVER_URL,
        capture_trace_content: bool = False,
    ):
        self.name = _display_name(name)
        self.server_url = server_url
        self.capture_trace_content = capture_trace_content
        self._data_dir = data_dir
        self._store = VifuAppStore(workspace)
        self._app: VifuAppRecord | None = None
        self._runtime: VifuRuntime | None = None
        self._registrations: list[tuple[str, AgentHandler, dict[str, Any]]] = []
        self._gateway: VifuGateway | None = None
        self._resources: list[Any] = []
        self._prepared_resources: set[int] = set()

    def agent(
        self,
        agent_id: str,
        handler: AgentHandler | None = None,
        *,
        name: str | None = None,
        endpoint: str | None = None,
        provider_id: str | None = None,
        capability: str = "chat",
        timeout_ms: int = 30_000,
        metadata: JsonValue = None,
    ) -> Callable[[AgentHandler], AgentHandler] | AgentHandler:
        """Registers a function or integration as an Agent."""

        def register(handler: AgentHandler) -> AgentHandler:
            agent_metadata = metadata
            if agent_metadata is None:
                agent_metadata = getattr(handler, "metadata", None)
            options = {
                "name": name,
                "endpoint": endpoint,
                "provider_id": provider_id,
                "capability": capability,
                "timeout_ms": timeout_ms,
                "metadata": agent_metadata,
            }
            self._registrations.append((agent_id, handler, options))
            if self._runtime is not None:
                self._runtime.agent(agent_id, handler, **options)
            if callable(getattr(handler, "prepare", None)):
                self._resources.append(handler)
            return handler

        return register if handler is None else register(handler)

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
        self._prepare_resources()
        return self.runtime.invoke(
            endpoint,
            input,
            session_id=session_id,
            metadata=metadata,
            timeout=timeout,
        )

    def connect(self, *, timeout: float = 20.0) -> VifuGateway:
        """Connects to the local Server and waits until the Gateway is ready."""
        try:
            self._prepare_resources()
            if self._gateway is None:
                runtime = self.runtime
                if _is_loopback_server(self.server_url):
                    assert self._app is not None
                self._gateway = runtime.connect_local(
                    server_url=self.server_url,
                    name=f"Python: {self.name}",
                    capture_trace_content=self.capture_trace_content,
                    app_id=self._app.app_id if self._app is not None else None,
                )
            self._gateway.wait_until_connected(timeout)
        except (OSError, RuntimeError, TimeoutError) as error:
            try:
                self.close()
            except Exception:
                pass
            raise ConnectionError(
                f"Vifu could not connect to the local Server at {self.server_url}: {error}"
            ) from error
        assert self._gateway is not None
        return self._gateway

    def run(
        self,
        main: Callable[["Vifu"], Any] | None = None,
        *,
        connect_timeout: float = 20.0,
    ) -> Any:
        """Runs application code with the App connected to Vifu."""
        self.connect(timeout=connect_timeout)
        print(f"Vifu Dashboard: {self.server_url.rstrip('/')}")
        print(f"App: {self.name} (connected)")
        try:
            if main is not None:
                return main(self)
            while True:
                time.sleep(3_600)
        except KeyboardInterrupt:
            pass
        finally:
            self.close()

    def close(self) -> None:
        """Stops resources that this Vifu application owns."""
        first_error: Exception | None = None
        if self._gateway is not None:
            gateway = self._gateway
            self._gateway = None
            try:
                gateway.close()
            except Exception as error:
                first_error = error
        for resource in reversed(self._resources):
            resource_id = id(resource)
            if resource_id not in self._prepared_resources:
                continue
            close = getattr(resource, "close", None)
            if callable(close):
                try:
                    close()
                except Exception as error:
                    first_error = first_error or error
            self._prepared_resources.discard(resource_id)
        if self._runtime is not None:
            runtime = self._runtime
            self._runtime = None
            try:
                runtime.close()
            except Exception as error:
                first_error = first_error or error
        if first_error is not None:
            raise first_error

    @property
    def runtime(self) -> VifuRuntime:
        """Returns the embedded Runtime for this stable Vifu App."""
        self._ensure_local_app()
        assert self._runtime is not None
        return self._runtime

    @property
    def app_id(self) -> str:
        """Returns the stable App ID assigned by the selected Vifu Server."""
        self._ensure_local_app()
        assert self._app is not None
        return self._app.app_id

    def _ensure_local_app(self) -> None:
        if self._runtime is not None:
            return
        if not _is_loopback_server(self.server_url):
            raise ValueError(
                "automatic App creation requires a loopback Vifu Server URL"
            )
        VifuServer.ensure(self.server_url)
        self._app = self._store.open(self.server_url, self.name)
        self._runtime = VifuRuntime(self._app.app_id, data_dir=self._data_dir)
        for agent_id, handler, options in self._registrations:
            self._runtime.agent(agent_id, handler, **options)

    def _prepare_resources(self) -> None:
        for resource in self._resources:
            resource_id = id(resource)
            if resource_id in self._prepared_resources:
                continue
            resource.prepare()
            self._prepared_resources.add(resource_id)

    def __enter__(self) -> "Vifu":
        self.connect()
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()


def _display_name(name: str) -> str:
    value = name.strip()
    if not value:
        raise ValueError("name must not be empty")
    return value


def _is_loopback_server(server_url: str) -> bool:
    parsed = urlparse(server_url)
    return parsed.scheme in {"http", "https"} and parsed.hostname in {
        "127.0.0.1",
        "localhost",
        "::1",
    }
