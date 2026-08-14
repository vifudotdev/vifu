"""Managed access to the complete local Vifu Server binary."""

from __future__ import annotations

import shutil
import ssl
import subprocess
import sys
import threading
import time
from collections import deque
from pathlib import Path
from typing import IO, Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from .gateway import DEFAULT_LOCAL_SERVER_URL


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
    ) -> VifuServer:
        resolved = _resolve_executable(executable)
        command = [resolved, *(arguments if arguments is not None else ["--no-browser"])]
        if arguments is None and profile is not None:
            command.extend(["--profile", profile])
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        output: deque[str] = deque(maxlen=100)
        thread = threading.Thread(target=_drain_output, args=(process.stdout, output), daemon=True)
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
        timeout: float = 15.0,
    ) -> VifuServer | None:
        """Reuses a ready local Server or starts the bundled Server."""
        if cls.is_ready(server_url):
            return None

        arguments = ["--no-browser", "--server-only"]
        if server_url.rstrip("/") != DEFAULT_LOCAL_SERVER_URL:
            arguments.extend(["-c", f"server.address={server_url.rstrip('/')}"])
        server = cls.start(
            executable=executable,
            arguments=arguments,
            wait_seconds=0.05,
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


def _local_tls_context(server_url: str) -> ssl.SSLContext | None:
    parsed = urlparse(server_url)
    if parsed.scheme == "https" and parsed.hostname in {"127.0.0.1", "localhost", "::1"}:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        return context
    return None
