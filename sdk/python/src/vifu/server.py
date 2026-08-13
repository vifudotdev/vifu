"""Managed access to the complete local Vifu Server binary."""

from __future__ import annotations

import shutil
import subprocess
import threading
import time
from collections import deque
from typing import IO, Any


class VifuServer:
    def __init__(self, process: subprocess.Popen[str], output: deque[str]):
        self._process = process
        self._output = output

    @classmethod
    def start(
        cls,
        *,
        executable: str = "vifu",
        profile: str | None = None,
        arguments: list[str] | None = None,
        wait_seconds: float = 1.0,
    ) -> VifuServer:
        resolved = shutil.which(executable)
        if resolved is None:
            raise FileNotFoundError(f"Vifu executable was not found: {executable}")
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
