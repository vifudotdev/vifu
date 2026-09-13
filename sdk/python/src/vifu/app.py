"""High-level API for a Python Agent application."""

from __future__ import annotations

import asyncio
import inspect
import json
import os
import tempfile
import threading
import time
from pathlib import Path
from types import MappingProxyType
from typing import Any, Callable, Mapping
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from .app_store import VifuAppRecord, VifuAppStore
from .gateway import DEFAULT_LOCAL_SERVER_URL, GatewayPairing, VifuGateway
from .providers import AppProvider, app_provider
from .runtime import AgentHandler, Invocation, JsonValue, VifuRuntime
from .server import VifuServer, VifuServerConfig

_MAX_MANAGED_CONTROL_BYTES = 64 * 1024


class Vifu:
    """Runs Python Agents in Vifu and connects them to the local Server."""

    def __init__(
        self,
        name: str,
        *,
        data_dir: str | Path | None = None,
        workspace: str | Path | None = None,
        server_url: str = DEFAULT_LOCAL_SERVER_URL,
        server_config: VifuServerConfig | None = None,
        capture_trace_content: bool = False,
    ):
        if server_config is not None:
            if server_url != DEFAULT_LOCAL_SERVER_URL and server_url != server_config.address:
                raise ValueError("server_url and server_config.address must match")
            server_url = server_config.address
        self.name = _display_name(name)
        self.server_url = server_url
        self.server_config = server_config or VifuServerConfig(address=server_url)
        self.capture_trace_content = capture_trace_content
        self._data_dir = data_dir
        self._store = VifuAppStore(workspace)
        self._app: VifuAppRecord | None = None
        self._runtime: VifuRuntime | None = None
        self._providers: dict[str, AppProvider] = {}
        self._registrations: list[tuple[str, AgentHandler, dict[str, Any]]] = []
        self._gateway: VifuGateway | None = None
        self._server: VifuServer | None = None
        self._resources: list[Any] = []
        self._prepared_resources: set[int] = set()
        self._foreground_lifecycles: list[Any] = []
        self._cloud_app_id = os.environ.get("VIFU_APP_ID", "").strip() or None
        self._cloud_app_slug = os.environ.get("VIFU_APP_SLUG", "").strip() or None
        self._managed_invocation_complete = threading.Event()

    @property
    def providers(self) -> Mapping[str, AppProvider]:
        """Returns this App's declared private Providers."""
        return MappingProxyType(self._providers)

    def provider(
        self,
        provider_id: str,
        implementation: Any,
        *,
        name: str | None = None,
        provider_type: str | None = None,
        capabilities: tuple[str, ...] | list[str] | None = None,
        settings: Mapping[str, Any] | None = None,
        resources: Mapping[str, str] | None = None,
    ) -> AppProvider:
        """Declares one reusable Provider in this Vifu App."""
        provider = app_provider(
            provider_id,
            implementation,
            name=name,
            provider_type=provider_type,
            capabilities=capabilities,
            settings=settings,
            resources=resources,
        )
        if provider.key in self._providers:
            raise ValueError(f"App Provider {provider.key} is already declared")
        self._providers[provider.key] = provider
        if all(existing is not provider for existing in self._resources):
            self._resources.append(provider)
        return provider

    def agent(
        self,
        agent_id: str,
        handler: AgentHandler | None = None,
        *,
        name: str | None = None,
        endpoint: str | None = None,
        provider_id: str | None = None,
        capability: str | None = None,
        timeout_ms: int | None = None,
        metadata: JsonValue = None,
        instructions: str | None = None,
        implementation: str | None = None,
        providers: Mapping[str, AppProvider | str] | None = None,
    ) -> Callable[[AgentHandler], AgentHandler] | AgentHandler:
        """Registers a function or integration as an Agent."""

        def register(handler: AgentHandler) -> AgentHandler:
            agent_metadata = metadata
            if agent_metadata is None:
                agent_metadata = getattr(
                    handler,
                    "vifu_metadata",
                    getattr(handler, "metadata", None),
                )
            resolved_implementation = implementation or getattr(
                handler,
                "vifu_implementation",
                None,
            )
            provider_bindings = self._agent_provider_bindings(providers)
            if resolved_implementation is not None or provider_bindings:
                if agent_metadata is None:
                    agent_metadata = {}
                if not isinstance(agent_metadata, dict):
                    raise ValueError(
                        "Agent metadata must be an object when implementation or Providers are set"
                    )
                agent_metadata = dict(agent_metadata)
                if resolved_implementation is not None:
                    resolved_implementation = resolved_implementation.strip()
                    if not resolved_implementation:
                        raise ValueError("Agent implementation must not be empty")
                    agent_metadata["implementation"] = resolved_implementation
                if provider_bindings:
                    agent_metadata["providerBindings"] = provider_bindings
            resolved_endpoint = endpoint or getattr(handler, "vifu_endpoint", None)
            options = {
                "name": name or getattr(handler, "vifu_name", None),
                "endpoint": resolved_endpoint,
                "provider_id": provider_id
                or getattr(handler, "vifu_provider_id", None),
                "capability": capability
                or getattr(handler, "vifu_capability", "chat"),
                "timeout_ms": timeout_ms
                or getattr(handler, "vifu_timeout_ms", 30_000),
                "metadata": agent_metadata,
                "instructions": instructions
                or getattr(handler, "vifu_instructions", None),
            }
            bind = getattr(handler, "vifu_bind", None)
            if callable(bind):
                bind(
                    self,
                    agent_id=agent_id,
                    endpoint=resolved_endpoint or agent_id,
                )
            self._registrations.append((agent_id, handler, options))
            if self._runtime is not None:
                self._runtime.agent(
                    agent_id,
                    handler,
                    _on_complete=self._managed_invocation_complete.set,
                    **options,
                )
            if callable(getattr(handler, "vifu_run", None)):
                self._foreground_lifecycles.append(handler)
            if any(
                callable(getattr(handler, method, None))
                for method in ("prepare", "vifu_run", "close")
            ) and all(existing is not handler for existing in self._resources):
                self._resources.append(handler)
            return handler

        return register if handler is None else register(handler)

    def _agent_provider_bindings(
        self,
        providers: Mapping[str, AppProvider | str] | None,
    ) -> dict[str, dict[str, str]]:
        bindings: dict[str, dict[str, str]] = {}
        for role, selected in (providers or {}).items():
            role = role.strip()
            if not role:
                raise ValueError("Agent Provider role must not be empty")
            provider = (
                self._providers.get(selected)
                if isinstance(selected, str)
                else selected
            )
            if provider is None or self._providers.get(provider.key) is not provider:
                raise ValueError(
                    f"Agent Provider {selected!r} must be declared by this App first"
                )
            capability = (
                role
                if role in provider.capabilities
                else provider.capabilities[0]
                if len(provider.capabilities) == 1
                else None
            )
            if capability is None:
                raise ValueError(
                    f"Provider {provider.key} has multiple capabilities; "
                    f"bind one of {', '.join(provider.capabilities)}"
                )
            bindings[role] = {
                "providerKey": provider.key,
                "capability": capability,
            }
        return bindings

    def _provider_descriptors(self) -> list[dict[str, Any]]:
        return [provider.descriptor() for provider in self._providers.values()]

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
        self._prepare_resources()
        try:
            if self._gateway is None:
                runtime = self.runtime
                pairing_path = os.environ.get("VIFU_GATEWAY_PAIRING_FILE", "").strip()
                if pairing_path:
                    pairing_code = _read_pairing_file(pairing_path)
                    pairing = GatewayPairing.parse(pairing_code)
                    self.server_url = pairing.server_url
                    self._gateway = runtime.connect(
                        pairing_code,
                        name=f"Python: {self.name}",
                        capture_trace_content=self.capture_trace_content,
                        providers=self._provider_descriptors(),
                    )
                else:
                    if _is_loopback_server(self.server_url):
                        assert self._app is not None
                    self._gateway = runtime.connect_local(
                        server_url=self.server_url,
                        name=f"Python: {self.name}",
                        capture_trace_content=self.capture_trace_content,
                        app_id=self._app.app_id if self._app is not None else None,
                        providers=self._provider_descriptors(),
                    )
            self._gateway.wait_until_connected(timeout)
            _notify_managed_ready()
        except Exception as error:
            try:
                self.close()
            except Exception:
                pass
            raise ConnectionError(
                f"Vifu could not connect to the Server at {self.server_url}: {error}"
            ) from error
        assert self._gateway is not None
        return self._gateway

    def run(
        self,
        main: Callable[["Vifu"], Any] | None = None,
        *,
        connect_timeout: float = 20.0,
    ) -> Any:
        """Runs a local entrypoint or serves one managed Endpoint invocation."""
        self.connect(timeout=connect_timeout)
        managed = bool(os.environ.get("VIFU_GATEWAY_PAIRING_FILE", "").strip())
        if not managed:
            print(f"Vifu Dashboard: {self.server_url.rstrip('/')}")
        print(f"App: {self.name} (connected)")
        try:
            if main is not None and not managed:
                result = main(self)
                if inspect.isawaitable(result):
                    result = asyncio.run(result)
                return result
            if len(self._foreground_lifecycles) > 1:
                raise RuntimeError(
                    "a Vifu App can have only one foreground Agent lifecycle"
                )
            if self._foreground_lifecycles:
                result = self._foreground_lifecycles[0].vifu_run()
                if inspect.isawaitable(result):
                    result = asyncio.run(result)
                return result
            if managed:
                while not self._managed_invocation_complete.wait(3_600):
                    pass
                return None
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
            if resource_id not in self._prepared_resources and not any(
                callable(getattr(resource, method, None))
                for method in ("vifu_bind", "vifu_run")
            ):
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
        if self._server is not None:
            server = self._server
            self._server = None
            try:
                server.close()
            except Exception as error:
                first_error = first_error or error
        if first_error is not None:
            raise first_error

    @property
    def runtime(self) -> VifuRuntime:
        """Returns the embedded Runtime for this stable Vifu App."""
        self._ensure_runtime()
        assert self._runtime is not None
        return self._runtime

    @property
    def app_id(self) -> str:
        """Returns the stable App ID assigned by the selected Vifu Server."""
        self._ensure_runtime()
        if self._app is not None:
            return self._app.app_id
        assert self._cloud_app_id is not None
        return self._cloud_app_id

    @property
    def slug(self) -> str:
        """Returns the App slug assigned by the selected Vifu Server."""
        self._ensure_runtime()
        if self._app is not None:
            return self._app.slug
        if self._cloud_app_slug is None:
            raise ValueError("VIFU_APP_SLUG is required for a managed App")
        return self._cloud_app_slug

    def _ensure_runtime(self) -> None:
        if self._runtime is not None:
            return
        pairing_path = os.environ.get("VIFU_GATEWAY_PAIRING_FILE", "").strip()
        if pairing_path:
            if self._cloud_app_id is None:
                raise ValueError("VIFU_APP_ID is required for a managed App")
            runtime_data_dir = self._data_dir
            if runtime_data_dir is None:
                workspace = os.environ.get("VIFU_EPHEMERAL_WORKSPACE", "").strip()
                root = Path(workspace) if workspace else Path(tempfile.gettempdir()) / "vifu-run"
                execution_id = os.environ.get("VIFU_MANAGED_EXECUTION_ID", "").strip()
                runtime_data_dir = root / (execution_id or "managed")
            self._runtime = VifuRuntime(self._cloud_app_id, data_dir=runtime_data_dir)
            for agent_id, handler, options in self._registrations:
                self._runtime.agent(
                    agent_id,
                    handler,
                    _on_complete=self._managed_invocation_complete.set,
                    **options,
                )
            return
        if not _is_loopback_server(self.server_url):
            raise ValueError(
                "automatic App creation requires a loopback Vifu Server URL"
            )
        self._server = VifuServer.ensure(
            self.server_url,
            profile=self.server_config.profile,
            overrides=self.server_config.overrides,
        )
        try:
            self._app = self._store.open(self.server_url, self.name)
            runtime_data_dir = self._data_dir
            if runtime_data_dir is None:
                runtime_data_dir = Path.home() / ".vifu" / "sdk" / "python" / self._app.app_id
            self._runtime = VifuRuntime(self._app.slug, data_dir=runtime_data_dir)
            for agent_id, handler, options in self._registrations:
                self._runtime.agent(agent_id, handler, **options)
        except Exception:
            if self._runtime is not None:
                runtime = self._runtime
                self._runtime = None
                runtime.close()
            if self._server is not None:
                server = self._server
                self._server = None
                server.close()
            raise

    def _prepare_resources(self) -> None:
        for resource in self._resources:
            resource_id = id(resource)
            if resource_id in self._prepared_resources:
                continue
            prepare = getattr(resource, "prepare", None)
            if callable(prepare):
                prepare()
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


def _secure_callback_url(value: str) -> bool:
    parsed = urlparse(value)
    if parsed.hostname is None or parsed.username is not None or parsed.password is not None:
        return False
    return parsed.scheme == "https" or (
        parsed.scheme == "http"
        and parsed.hostname in {"127.0.0.1", "localhost", "::1"}
    )


def _read_pairing_file(path: str | Path) -> str:
    source = Path(path)
    data = source.read_bytes()
    if len(data) > 64 * 1024:
        raise ValueError("VIFU_GATEWAY_PAIRING_FILE exceeds 64 KiB")
    try:
        text = data.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        raise ValueError("VIFU_GATEWAY_PAIRING_FILE must be UTF-8") from error
    if not text:
        raise ValueError("VIFU_GATEWAY_PAIRING_FILE is empty")
    if text.startswith("{"):
        try:
            value = json.loads(text)
        except json.JSONDecodeError as error:
            raise ValueError("VIFU_GATEWAY_PAIRING_FILE contains invalid JSON") from error
        if not isinstance(value, dict):
            raise ValueError("VIFU_GATEWAY_PAIRING_FILE must contain an object")
        text = str(value.get("pairingCode") or value.get("pairing_code") or "").strip()
        if not text:
            raise ValueError("VIFU_GATEWAY_PAIRING_FILE is missing pairingCode")
    return text


def _notify_managed_ready() -> None:
    control_file = os.environ.get("VIFU_MANAGED_CONTROL_FILE", "").strip()
    if not control_file:
        return
    source = Path(control_file)
    try:
        data = source.read_bytes()
    except OSError as error:
        raise RuntimeError("managed Vifu control data is unavailable") from error
    if len(data) > _MAX_MANAGED_CONTROL_BYTES:
        raise ValueError("managed Vifu control data exceeds 64 KiB")
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("managed Vifu control data is invalid") from error
    if not isinstance(value, dict):
        raise ValueError("managed Vifu control data must be an object")
    ready_url = value.get("readyUrl")
    token = value.get("token")
    execution_id = value.get("executionId")
    if not all(isinstance(item, str) and item for item in (ready_url, token, execution_id)):
        raise ValueError("managed Vifu control data is incomplete")
    if not _secure_callback_url(ready_url):
        raise ValueError("managed Vifu ready URL must use HTTPS or loopback HTTP")
    request = Request(
        ready_url,
        data=json.dumps(
            {"executionId": execution_id},
            separators=(",", ":"),
        ).encode("utf-8"),
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urlopen(request, timeout=10) as response:
            if response.status < 200 or response.status >= 300:
                raise RuntimeError("managed Vifu ready notification was rejected")
    except OSError as error:
        raise RuntimeError("managed Vifu ready notification failed") from error
