"""Persistent pairing and lifecycle support for the embedded Vifu Gateway."""

from __future__ import annotations

import base64
import json
import os
import platform
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any
from urllib.parse import parse_qs, urlparse

from . import vifu_mobile_ffi as native

if TYPE_CHECKING:
    from .runtime import VifuRuntime


@dataclass(frozen=True)
class GatewayPairing:
    server_url: str
    enrollment_token: str
    certificate_der: bytes | None = None

    @classmethod
    def parse(cls, code: str) -> GatewayPairing:
        parsed = urlparse(code.strip())
        if parsed.scheme == "vifu" and parsed.netloc == "gateway" and parsed.path == "/enroll":
            values = parse_qs(parsed.query)
        elif (
            parsed.scheme == "https"
            and parsed.netloc in {"vifu.ai", "www.vifu.ai"}
            and parsed.path == "/pair"
        ):
            values = parse_qs(parsed.fragment)
        else:
            raise ValueError("pairing code must use vifu://gateway/enroll or https://vifu.ai/pair")
        server_url = _one(values, "server").rstrip("/")
        token = _one(values, "token")
        if not token.startswith("vifu_ge_"):
            raise ValueError("pairing code has an invalid enrollment token")
        encoded_certificate = values.get("certificate", [None])[0]
        certificate = None
        if encoded_certificate:
            padding = "=" * ((4 - len(encoded_certificate) % 4) % 4)
            certificate = base64.b64decode(
                encoded_certificate + padding,
                altchars=b"-_",
                validate=True,
            )
        return cls(server_url=server_url, enrollment_token=token, certificate_der=certificate)


@dataclass
class _GatewayCredentials:
    server_url: str
    machine_private_key: str
    device_token: str | None = None
    certificate_der_base64: str | None = None


class VifuGateway:
    def __init__(
        self,
        native_gateway: native.VifuEmbeddedGateway,
        credentials_path: Path,
        credentials: _GatewayCredentials,
    ):
        self._native = native_gateway
        self._credentials_path = credentials_path
        self._credentials = credentials

    @classmethod
    def connect(
        cls,
        *,
        runtime: VifuRuntime,
        pairing_code: str | None,
        name: str | None,
        capture_trace_content: bool,
    ) -> VifuGateway:
        credentials_path = runtime.data_dir / "gateway.json"
        stored = _load_credentials(credentials_path)
        pairing = GatewayPairing.parse(pairing_code) if pairing_code is not None else None
        if pairing is None and stored is None:
            raise ValueError("pairing_code is required for the first Gateway connection")
        server_url = pairing.server_url if pairing is not None else stored.server_url
        if stored is None or stored.server_url != server_url:
            identity = native.generate_vifu_gateway_identity()
            stored = _GatewayCredentials(
                server_url=server_url,
                machine_private_key=identity.private_key,
            )
        if pairing is not None and pairing.certificate_der is not None:
            stored.certificate_der_base64 = base64.b64encode(pairing.certificate_der).decode("ascii")
        certificate = (
            base64.b64decode(stored.certificate_der_base64)
            if stored.certificate_der_base64 is not None
            else None
        )
        metadata = {
            "name": name or f"Python on {platform.node() or 'local device'}",
            "platform": platform.system().lower(),
            "runtime": "python",
            "sdkVersion": "0.1.12",
        }
        gateway = native.VifuEmbeddedGateway(
            runtime._native,
            native.VifuEmbeddedGatewayConfig(
                server_url=server_url,
                runtime_database_path=str(runtime.database_path),
                server_certificate_der=certificate,
                gateway_metadata_json=json.dumps(metadata, separators=(",", ":")),
            ),
        )
        enrollment_token = pairing.enrollment_token if pairing is not None else None
        if capture_trace_content:
            gateway.start_with_monitor_io(
                stored.machine_private_key,
                stored.device_token,
                enrollment_token,
                True,
            )
        else:
            gateway.start(stored.machine_private_key, stored.device_token, enrollment_token)
        _save_credentials(credentials_path, stored)
        return cls(gateway, credentials_path, stored)

    @property
    def state(self) -> str:
        return self.refresh().state.name.lower()

    def refresh(self) -> native.VifuEmbeddedGatewayStatus:
        status = self._native.status()
        if status.authorization is not None and status.authorization.device_token != self._credentials.device_token:
            self._credentials.device_token = status.authorization.device_token
            _save_credentials(self._credentials_path, self._credentials)
        return status

    def wait_until_connected(self, timeout: float = 20.0) -> native.VifuEmbeddedGatewayStatus:
        deadline = time.monotonic() + timeout
        while True:
            status = self.refresh()
            if status.state == native.VifuEmbeddedGatewayState.CONNECTED:
                return status
            if status.state in (
                native.VifuEmbeddedGatewayState.AUTHORIZATION_REQUIRED,
                native.VifuEmbeddedGatewayState.FAILED,
            ):
                raise RuntimeError(status.last_error or f"Gateway is {status.state.name.lower()}")
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Gateway did not connect within {timeout:.1f} seconds")
            time.sleep(0.05)

    def close(self) -> None:
        self.refresh()
        self._native.stop()

    def __enter__(self) -> VifuGateway:
        return self

    def __exit__(self, *_args: Any) -> None:
        self.close()


def _one(values: dict[str, list[str]], key: str) -> str:
    value = values.get(key, [""])[0].strip()
    if not value:
        raise ValueError(f"pairing code is missing {key}")
    return value


def _load_credentials(path: Path) -> _GatewayCredentials | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    return _GatewayCredentials(**value)


def _save_credentials(path: Path, credentials: _GatewayCredentials) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(asdict(credentials), indent=2), encoding="utf-8")
    if os.name != "nt":
        temporary.chmod(0o600)
    temporary.replace(path)
