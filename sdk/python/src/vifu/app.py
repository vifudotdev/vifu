"""High-level API for a Python Agent application."""

from __future__ import annotations

import hashlib
import json
import re
import time
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlparse

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
        server_url: str = DEFAULT_LOCAL_SERVER_URL,
        capture_trace_content: bool = False,
    ):
        self.name = _display_name(name)
        self.runtime = VifuRuntime(_runtime_id(self.name), data_dir=data_dir)
        self.server_url = server_url
        self.capture_trace_content = capture_trace_content
        self._gateway: VifuGateway | None = None
        self._server: VifuServer | None = None
        self._resources: list[Any] = []
        self._prepared_resources: set[int] = set()
        self._endpoints: list[str] = []

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
            self.runtime.agent(
                agent_id,
                handler,
                name=name,
                endpoint=endpoint,
                provider_id=provider_id,
                capability=capability,
                timeout_ms=timeout_ms,
                metadata=agent_metadata,
            )
            if callable(getattr(handler, "prepare", None)):
                self._resources.append(handler)
            self._endpoints.append(endpoint or agent_id)
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
                if _is_loopback_server(self.server_url):
                    self._server = VifuServer.ensure(self.server_url)
                self._gateway = self.runtime.connect_local(
                    server_url=self.server_url,
                    name=f"Python: {self.name}",
                    capture_trace_content=self.capture_trace_content,
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
        *,
        endpoint: str | None = None,
        session_id: str = "terminal-chat",
        connect_timeout: float = 20.0,
    ) -> None:
        """Runs an Agent for terminal prompts and remote calls."""
        selected_endpoint = endpoint or self._single_endpoint()
        self.connect(timeout=connect_timeout)
        print(f"Vifu Dashboard: {self.server_url.rstrip('/')}")
        print(f"Agent: {self.name} (connected)")
        try:
            self._run_terminal(selected_endpoint, session_id)
        except KeyboardInterrupt:
            pass
        finally:
            self.close()

    def serve(self, *, connect_timeout: float = 20.0) -> None:
        """Serves remote Agent calls until the process stops."""
        self.connect(timeout=connect_timeout)
        print(f"Vifu Dashboard: {self.server_url.rstrip('/')}")
        print(f"Agent: {self.name} (connected)")
        try:
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
        if self._server is not None:
            server = self._server
            self._server = None
            try:
                server.close()
            except Exception as error:
                first_error = first_error or error
        if first_error is not None:
            raise first_error

    def _prepare_resources(self) -> None:
        for resource in self._resources:
            resource_id = id(resource)
            if resource_id in self._prepared_resources:
                continue
            resource.prepare()
            self._prepared_resources.add(resource_id)

    def _run_terminal(self, endpoint: str, session_id: str) -> None:
        print("Enter /quit to stop the Agent.")
        while True:
            try:
                prompt = input("\nYou > ").strip()
            except EOFError:
                return
            if prompt == "/quit":
                return
            if not prompt:
                continue
            try:
                result = self.invoke(
                    endpoint,
                    {"prompt": prompt},
                    session_id=session_id,
                )
            except Exception as error:
                print(f"Agent error > {error}")
                continue
            print(f"Agent > {_terminal_output(result.output)}")
            print(f"Trace > {result.invocation_id}")

    def _single_endpoint(self) -> str:
        unique = list(dict.fromkeys(self._endpoints))
        if len(unique) != 1:
            raise ValueError("endpoint is required when the application has multiple Agents")
        return unique[0]

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


def _is_loopback_server(server_url: str) -> bool:
    parsed = urlparse(server_url)
    return parsed.scheme in {"http", "https"} and parsed.hostname in {
        "127.0.0.1",
        "localhost",
        "::1",
    }


def _terminal_output(value: JsonValue) -> str:
    if isinstance(value, dict) and isinstance(value.get("text"), str):
        return value["text"]
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False)
