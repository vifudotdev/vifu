"""Managed access to the complete local Vifu Server binary."""

from __future__ import annotations

import json
import shutil
import ssl
import subprocess
import sys
import threading
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import IO, Any, Mapping
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from .gateway import DEFAULT_LOCAL_SERVER_URL


ServerConfigValue = str | int | float | bool


@dataclass(frozen=True)
class VifuServerConfig:
    """Startup configuration for a local Server managed by a Python App."""

    address: str = DEFAULT_LOCAL_SERVER_URL
    profile: str | None = None
    overrides: Mapping[str, ServerConfigValue] = field(default_factory=dict)


class VifuServer:
    def __init__(self, process: subprocess.Popen[str], output: deque[str]):
        self._process = process
        self._output = output

    @classmethod
    def start(
        cls,
        *,
        executable: str | None = None,
        profile: str | None = None,
        arguments: list[str] | None = None,
        wait_seconds: float = 1.0,
        shared: bool = False,
    ) -> VifuServer:
        resolved = _resolve_executable(executable)
        command = [resolved, *(arguments if arguments is not None else ["--no-browser"])]
        if arguments is None and profile is not None:
            command.extend(["--profile", profile])
        process_options: dict[str, Any] = {
            "stdout": subprocess.DEVNULL if shared else subprocess.PIPE,
            "stderr": subprocess.DEVNULL if shared else subprocess.STDOUT,
            "text": True,
            "bufsize": 1,
        }
        if shared:
            if sys.platform == "win32":
                process_options["creationflags"] = (
                    subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS
                )
            else:
                process_options["start_new_session"] = True
        process = subprocess.Popen(command, **process_options)
        output: deque[str] = deque(maxlen=100)
        if process.stdout is not None:
            thread = threading.Thread(
                target=_drain_output,
                args=(process.stdout, output),
                daemon=True,
            )
            thread.start()
        deadline = time.monotonic() + wait_seconds
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError("Vifu Server stopped during startup:\n" + "".join(output))
            time.sleep(0.02)
        return cls(process, output)

    @classmethod
    def ensure(
        cls,
        server_url: str = DEFAULT_LOCAL_SERVER_URL,
        *,
        executable: str | None = None,
        profile: str | None = None,
        overrides: Mapping[str, ServerConfigValue] | None = None,
        timeout: float = 15.0,
    ) -> VifuServer | None:
        """Reuses a ready personal Server or starts one independently."""
        if cls.is_ready(server_url):
            return None

        arguments = ["--no-browser", "--server-only"]
        if profile is not None:
            arguments.extend(["--profile", profile])
        if profile is not None or server_url.rstrip("/") != DEFAULT_LOCAL_SERVER_URL:
            arguments.extend(["-c", f"server.address={server_url.rstrip('/')}"])
        for key, value in (overrides or {}).items():
            arguments.extend(["-c", f"{_config_key(key)}={_config_value(value)}"])
        server = cls.start(
            executable=executable,
            arguments=arguments,
            wait_seconds=0.05,
            shared=True,
        )
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if cls.is_ready(server_url):
                return server if server.running else None
            if not server.running:
                output = server.recent_output.strip()
                server.close()
                if cls.is_ready(server_url):
                    return None
                detail = f"\n{output}" if output else ""
                raise RuntimeError(f"The bundled Vifu Server stopped during startup.{detail}")
            time.sleep(0.05)
        output = server.recent_output.strip()
        server.close()
        detail = f"\n{output}" if output else ""
        raise TimeoutError(f"The Vifu Server did not become ready at {server_url}.{detail}")

    @staticmethod
    def is_ready(server_url: str, *, timeout: float = 0.2) -> bool:
        request = Request(f"{server_url.rstrip('/')}/health", method="GET")
        context = _local_tls_context(server_url)
        try:
            with urlopen(request, timeout=timeout, context=context) as response:
                return 200 <= response.status < 300
        except (HTTPError, URLError, OSError, TimeoutError, ValueError):
            return False

    @property
    def running(self) -> bool:
        return self._process.poll() is None

    @property
    def recent_output(self) -> str:
        return "".join(self._output)

    def close(self, timeout: float = 5.0) -> None:
        if self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=timeout)
        if self._process.stdout is not None:
            self._process.stdout.close()

    def __enter__(self) -> VifuServer:
        return self

    def __exit__(self, *_args: Any) -> None:
        self.close()


def _drain_output(stream: IO[str] | None, output: deque[str]) -> None:
    if stream is None:
        return
    for line in stream:
        output.append(line)


def _resolve_executable(executable: str | None) -> str:
    if executable is not None:
        path = Path(executable).expanduser()
        if path.parent != Path(".") or path.is_absolute():
            if path.is_file():
                return str(path)
        else:
            resolved = shutil.which(executable)
            if resolved is not None:
                return resolved
        raise FileNotFoundError(f"Vifu executable was not found: {executable}")

    name = "vifu.exe" if sys.platform == "win32" else "vifu"
    bundled = Path(__file__).resolve().parent / "_bin" / name
    if bundled.is_file():
        return str(bundled)
    resolved = shutil.which(name)
    if resolved is not None:
        return resolved
    raise FileNotFoundError(
        "The Vifu Server is not in this Python package. "
        "Install an official Vifu wheel for this platform."
    )


def _config_key(value: str) -> str:
    key = value.strip()
    if not key or any(
        not segment
        or not segment.replace("_", "").replace("-", "").isalnum()
        for segment in key.split(".")
    ):
        raise ValueError(f"invalid Vifu Server configuration key: {value!r}")
    if key == "server.address":
        raise ValueError("set VifuServerConfig.address instead of overriding server.address")
    return key


def _config_value(value: ServerConfigValue) -> str:
    return json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"))


def _local_tls_context(server_url: str) -> ssl.SSLContext | None:
    parsed = urlparse(server_url)
    if parsed.scheme == "https" and parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        return context
    return None
