"""Project-scoped bindings to Apps in a personal Vifu Server."""

from __future__ import annotations

import json
import os
import ssl
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


@dataclass(frozen=True)
class VifuAppRecord:
    app_id: str
    slug: str
    name: str


class VifuAppStore:
    """Opens one project App in the user's personal Vifu Server."""

    def __init__(self, workspace: str | os.PathLike[str] | None = None):
        root = Path(workspace) if workspace is not None else Path.cwd()
        self.workspace = root.resolve()
        self.manifest_path = self.workspace / ".vifu" / "app.json"

    def open(self, server_url: str, name: str) -> VifuAppRecord:
        server_url = server_url.rstrip("/")
        manifest = self._load()
        binding = manifest.get("servers", {}).get(server_url)
        app_id = binding.get("appId") if isinstance(binding, dict) else None
        opened = self._request(
            server_url,
            "v1/local/apps/open",
            method="POST",
            body={"name": name, "appId": app_id},
        )
        app = opened.get("app")
        if not isinstance(app, dict):
            raise RuntimeError("Vifu Server did not return the created App")
        record = _record(app)
        servers = manifest.setdefault("servers", {})
        servers[server_url] = {
            "appId": record.app_id,
            "slug": record.slug,
        }
        manifest["name"] = name
        self._save(manifest)
        return record

    def _request(
        self,
        server_url: str,
        path: str,
        *,
        method: str = "GET",
        body: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = Request(
            f"{server_url}/{path}",
            data=data,
            method=method,
            headers={"Content-Type": "application/json"} if data is not None else {},
        )
        try:
            with urlopen(request, timeout=5.0, context=_tls_context(server_url)) as response:
                payload = json.loads(response.read())
        except HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(
                f"Vifu Server rejected the App request ({error.code}): {detail}"
            ) from error
        except (URLError, OSError, TimeoutError, ValueError) as error:
            raise RuntimeError(f"Vifu Server App request failed: {error}") from error
        if not isinstance(payload, dict):
            raise RuntimeError("Vifu Server returned an invalid App response")
        return payload

    def _load(self) -> dict[str, Any]:
        try:
            payload = json.loads(self.manifest_path.read_text(encoding="utf-8"))
        except FileNotFoundError:
            return {"schemaVersion": 1, "servers": {}}
        except (OSError, ValueError) as error:
            raise RuntimeError(f"Vifu App manifest is invalid: {self.manifest_path}") from error
        if not isinstance(payload, dict) or payload.get("schemaVersion") != 1:
            raise RuntimeError(f"Vifu App manifest is invalid: {self.manifest_path}")
        if not isinstance(payload.get("servers"), dict):
            raise RuntimeError(f"Vifu App manifest is invalid: {self.manifest_path}")
        return payload

    def _save(self, manifest: dict[str, Any]) -> None:
        self.manifest_path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.manifest_path.with_suffix(".tmp")
        temporary.write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        temporary.replace(self.manifest_path)


def _record(value: dict[str, Any]) -> VifuAppRecord:
    app_id = value.get("appId")
    slug = value.get("slug")
    name = value.get("name")
    if not all(isinstance(item, str) and item for item in (app_id, slug, name)):
        raise RuntimeError("Vifu Server returned an invalid App record")
    return VifuAppRecord(app_id=app_id, slug=slug, name=name)


def _tls_context(server_url: str) -> ssl.SSLContext | None:
    if server_url.startswith("https://127.0.0.1") or server_url.startswith(
        "https://localhost"
    ):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        return context
    return None
