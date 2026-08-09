#!/usr/bin/env python3
"""Reproducible readiness and roster checks for the pinned StarDojo demo."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import secrets
import shlex
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
ROSTER_PATH = ROOT / "roster.json"
REPOSITORY_ROOT = ROOT.parent.parent
DEFAULT_ADMIN_ENV = REPOSITORY_ROOT / "npm-packages" / "dashboard" / ".env.local"
DEFAULT_DEMO_ENV = REPOSITORY_ROOT / "target" / "stardew_valley" / ".env.local"
STARDOJO_API_KEY_ENV = "OA_OPENAI_KEY"
OPENAI_BASE_URL_ENV = "OPENAI_BASE_URL"
STARDEW_APP_PATH_ENV = "STARDEW_APP_PATH"


@dataclass(frozen=True)
class Check:
    name: str
    status: str
    detail: str


@dataclass(frozen=True)
class ProjectBootstrap:
    project_id: str
    project_slug: str
    gateway_id: str
    provider_key: str
    embedding_provider_key: str
    models: tuple[str, ...]
    physical_agent_count: int
    physical_endpoint_count: int
    created_endpoints: int
    task_model_count: int
    created_task_profiles: int
    updated_task_profiles: int


class VifuApiError(RuntimeError):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(f"Vifu API returned {status} ({code}): {message}")
        self.status = status
        self.code = code


class VifuClient:
    def __init__(
        self,
        server_url: str,
        credential: str,
        *,
        authorization_scheme: str = "Vifu",
    ) -> None:
        self.server_url = server_url.rstrip("/")
        self.credential = credential
        self.authorization_scheme = authorization_scheme

    def request(
        self,
        path: str,
        *,
        method: str = "GET",
        body: dict[str, Any] | None = None,
        timeout: float = 30,
    ) -> dict[str, Any]:
        data = json.dumps(body).encode() if body is not None else None
        headers = {
            "Accept": "application/json",
            "Authorization": f"{self.authorization_scheme} {self.credential}",
        }
        if data is not None:
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"{self.server_url}/{path.lstrip('/')}",
            data=data,
            headers=headers,
            method=method,
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout) as response:
                raw = response.read()
        except urllib.error.HTTPError as error:
            try:
                payload = json.loads(error.read())
            except (ValueError, OSError):
                payload = {}
            api_error = payload.get("error", {})
            raise VifuApiError(
                error.code,
                str(api_error.get("code", "unknown")),
                str(api_error.get("message", "request failed")),
            ) from error
        if not raw:
            return {}
        try:
            return json.loads(raw)
        except ValueError as error:
            raise RuntimeError("Vifu API returned invalid JSON") from error


def read_env_file(path: Path) -> dict[str, str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError:
        return {}
    values: dict[str, str] = {}
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        key = key.strip()
        value = value.strip()
        try:
            parsed = shlex.split(value, comments=False, posix=True)
        except ValueError:
            parsed = []
        if len(parsed) == 1:
            value = parsed[0]
        values[key] = value
    return values


def write_private_env(path: Path, values: dict[str, str]) -> None:
    for key, value in values.items():
        if not key or any(character in key for character in "=\r\n"):
            raise ValueError("environment variable names must be non-empty single-line keys")
        if any(character in value for character in "\r\n"):
            raise ValueError(f"{key} must be a single-line value")
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            for key, value in values.items():
                stream.write(f"{key}={shlex.quote(value)}\n")
        os.replace(temporary, path)
        path.chmod(0o600)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def stardojo_env_values(
    *,
    existing: dict[str, str],
    project_api_key: str,
    project_base_url: str,
) -> dict[str, str]:
    values = {
        STARDOJO_API_KEY_ENV: project_api_key,
        OPENAI_BASE_URL_ENV: project_base_url,
    }
    stardew_app_path = existing.get(STARDEW_APP_PATH_ENV) or os.getenv(
        STARDEW_APP_PATH_ENV
    )
    if stardew_app_path:
        values[STARDEW_APP_PATH_ENV] = stardew_app_path
    return values


def provider_agents(gateway: dict[str, Any], provider_key: str) -> list[dict[str, Any]]:
    return [
        agent
        for agent in gateway.get("agents", [])
        if agent.get("status") in {None, "connected", "online"}
        and agent.get("metadata", {}).get("providerKey") == provider_key
    ]


def binding_provider_key(binding: dict[str, Any]) -> str:
    config = binding.get("config", {})
    if isinstance(config, dict) and config.get("providerKey"):
        return str(config["providerKey"])
    return str(binding.get("providerKey") or binding.get("provider") or "")


def select_provider_gateway(
    gateways: list[dict[str, Any]],
    provider_key: str,
    requested_gateway_id: str | None,
    current_gateway_id: str | None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    candidates = [
        (gateway, provider_agents(gateway, provider_key))
        for gateway in gateways
        if gateway.get("status") == "connected"
    ]
    candidates = [(gateway, agents) for gateway, agents in candidates if agents]
    preferred = requested_gateway_id or current_gateway_id
    if preferred:
        for gateway, agents in candidates:
            if gateway.get("gatewayId") == preferred:
                return gateway, agents
        if requested_gateway_id:
            raise RuntimeError(
                f"gateway {requested_gateway_id} is not connected with provider {provider_key}"
            )
    if len(candidates) == 1:
        return candidates[0]
    if not candidates:
        raise RuntimeError(f"no connected Gateway advertises provider {provider_key}")
    ids = ", ".join(str(gateway.get("gatewayId")) for gateway, _ in candidates)
    raise RuntimeError(f"multiple Gateways advertise provider {provider_key}: {ids}; use --gateway-id")


def select_provider_agent(
    agents: list[dict[str, Any]], provider_key: str
) -> dict[str, Any]:
    exact = [agent for agent in agents if str(agent.get("id")) == provider_key]
    if len(exact) == 1:
        return exact[0]
    if len(agents) == 1:
        return agents[0]
    ids = ", ".join(str(agent.get("id")) for agent in agents)
    raise RuntimeError(
        f"provider {provider_key} advertises multiple physical Agents: {ids}"
    )


def task_profile_name(model: str) -> str:
    prefix = "stardew-valley-"
    if not model.startswith(prefix):
        raise ValueError(f"invalid Stardew Valley task model: {model}")
    category_and_id = model.removeprefix(prefix)
    category, separator, task_id = category_and_id.rpartition("-")
    if not separator or not category or not task_id.isdigit():
        raise ValueError(f"invalid Stardew Valley task model: {model}")
    return f"Stardew Valley {category}/{task_id}"


def task_profile_configuration(
    *,
    model: str,
    gateway_id: str,
    chat_provider_key: str,
    chat_resource_id: str,
    embedding_provider_key: str,
    embedding_resource_id: str,
) -> dict[str, Any]:
    source = {
        "type": "vifu-runtime",
        "managed": True,
        "gatewayId": gateway_id,
        "providerKey": chat_provider_key,
        "resourceId": chat_resource_id,
        "embeddingProviderKey": embedding_provider_key,
        "embeddingResourceId": embedding_resource_id,
        "integration": "stardew_valley",
        "taskModel": model,
    }
    return {
        "persona": {"files": {}},
        "runtime": {"requestTimeoutMs": 120_000},
        "presentation": {
            "integration": "stardew_valley",
            "taskModel": model,
        },
        "source": source,
        "capabilities": [
            {
                "kind": "chat",
                "providerType": "vifu-runtime",
                "providerKey": chat_provider_key,
                "resourceId": chat_resource_id,
                "config": {
                    "gatewayId": gateway_id,
                    "source": "vifu-runtime-discovery",
                },
                "inputSchema": {},
                "outputSchema": {},
            },
            {
                "kind": "embedding",
                "providerType": "vifu-runtime",
                "providerKey": embedding_provider_key,
                "resourceId": embedding_resource_id,
                "config": {
                    "gatewayId": gateway_id,
                    "source": "vifu-runtime-discovery",
                },
                "inputSchema": {},
                "outputSchema": {},
            }
        ],
    }


def active_profile_matches(
    detail: dict[str, Any], desired: dict[str, Any]
) -> bool:
    profile = detail.get("profile", {})
    active_version_id = str(profile.get("activeVersionId", ""))
    active = next(
        (
            item
            for item in detail.get("versions", [])
            if str(item.get("version", {}).get("id", "")) == active_version_id
        ),
        None,
    )
    if active is None:
        return False
    version = active.get("version", {})
    if any(
        version.get(field) != desired[field]
        for field in ("persona", "runtime", "presentation", "source")
    ):
        return False
    capability_fields = (
        "kind",
        "providerType",
        "providerKey",
        "resourceId",
        "config",
        "inputSchema",
        "outputSchema",
    )
    actual_capabilities = [
        {field: capability.get(field) for field in capability_fields}
        for capability in active.get("capabilities", [])
    ]
    return actual_capabilities == desired["capabilities"]


def ensure_project_configuration(
    client: VifuClient,
    *,
    project_slug: str,
    project_name: str,
    chat_provider_key: str,
    embedding_provider_key: str,
    requested_gateway_id: str | None,
    task_models: list[str],
) -> ProjectBootstrap:
    expected_task_models = tuple(dict.fromkeys(task_models))
    if len(expected_task_models) != len(task_models):
        raise ValueError("Stardew Valley task models must be unique")
    if not expected_task_models:
        raise ValueError("at least one Stardew Valley task model is required")
    for model in expected_task_models:
        task_profile_name(model)

    projects = client.request("/v1/projects").get("projects", [])
    project = next(
        (candidate for candidate in projects if candidate.get("slug") == project_slug),
        None,
    )
    gateways = client.request("/v1/agent-gateways").get("agentGateways", [])
    gateway, chat_agents = select_provider_gateway(
        gateways,
        chat_provider_key,
        requested_gateway_id,
        project.get("gatewayId") if project else None,
    )
    gateway_id = str(gateway["gatewayId"])
    chat_physical_agent = select_provider_agent(chat_agents, chat_provider_key)
    chat_resource_id = str(chat_physical_agent["id"])
    if embedding_provider_key == chat_provider_key:
        embedding_agents = chat_agents
    else:
        _embedding_gateway, embedding_agents = select_provider_gateway(
            gateways,
            embedding_provider_key,
            gateway_id,
            gateway_id,
        )
    embedding_physical_agent = select_provider_agent(
        embedding_agents, embedding_provider_key
    )
    embedding_resource_id = str(embedding_physical_agent["id"])
    if project is None:
        project = client.request(
            "/v1/projects",
            method="POST",
            body={"name": project_name, "slug": project_slug},
        )["project"]
    project_id = str(project["id"])
    project = client.request(
        f"/v1/projects/{urllib.parse.quote(project_id)}",
        method="PATCH",
        body={"gatewayId": gateway_id},
    )["project"]

    project_path = f"/v1/project/{urllib.parse.quote(project_slug)}"
    deployments = client.request(f"{project_path}/deployments").get("deployments", [])
    deployment = next(
        (candidate for candidate in deployments if candidate.get("isPrimary") is True),
        None,
    )
    if deployment is None:
        raise RuntimeError("Project does not have a primary runtime deployment")
    if deployment.get("remoteInvocationEnabled") is not True:
        deployment_name = urllib.parse.quote(str(deployment["name"]))
        client.request(
            f"{project_path}/deployments/{deployment_name}",
            method="PATCH",
            body={"remoteInvocationEnabled": True},
        )

    provider_path = f"/v1/project/{urllib.parse.quote(project_slug)}/providers"
    for provider_key in dict.fromkeys([chat_provider_key, embedding_provider_key]):
        providers = client.request(provider_path).get("providers", [])
        provider = next(
            (
                candidate
                for candidate in providers
                if candidate.get("sourceKind") == "custom"
                and candidate.get("sourceKey") == provider_key
            ),
            None,
        )
        if provider is None:
            client.request(
                provider_path,
                method="POST",
                body={"source": {"kind": "custom", "key": provider_key}},
            )
        else:
            connection_key = urllib.parse.quote(str(provider["providerKey"]))
            client.request(f"{provider_path}/{connection_key}/test", method="POST")

    profiles = client.request(f"{project_path}/profiles").get("profiles", [])
    bindings = client.request(f"{project_path}/bindings").get("bindings", [])
    endpoints = client.request(f"{project_path}/endpoints").get("endpoints", [])
    profiles_by_id = {profile["id"]: profile for profile in profiles}
    provider_agent_ids = {
        (chat_provider_key, str(agent["id"])) for agent in chat_agents
    } | {
        (embedding_provider_key, str(agent["id"])) for agent in embedding_agents
    }
    provider_bindings = [
        binding
        for binding in bindings
        if binding.get("gatewayId") == gateway_id
        and (binding_provider_key(binding), str(binding.get("agentId")))
        in provider_agent_ids
        and binding.get("profileId") in profiles_by_id
    ]
    if not provider_bindings:
        raise RuntimeError("provider refresh did not create any Project Agent bindings")

    endpoint_pairs = {
        (endpoint.get("profileId"), endpoint.get("bindingId")) for endpoint in endpoints
    }
    provider_binding_pairs = {
        (binding["profileId"], binding["id"]) for binding in provider_bindings
    }
    created_endpoints = 0
    for binding in provider_bindings:
        pair = (binding["profileId"], binding["id"])
        if pair in endpoint_pairs:
            continue
        profile = profiles_by_id[binding["profileId"]]
        client.request(
            f"{project_path}/endpoints",
            method="POST",
            body={
                "name": profile["name"],
                "slug": profile["slug"],
                "profileId": profile["id"],
                "bindingId": binding["id"],
                "requestTimeoutMs": 120_000,
            },
        )["endpoint"]
        endpoint_pairs.add(pair)
        created_endpoints += 1

    task_profiles_by_slug = {
        str(profile["slug"]): profile
        for profile in profiles
        if str(profile.get("slug")) in expected_task_models
    }
    created_task_profiles = 0
    updated_task_profiles = 0
    for model in expected_task_models:
        desired = task_profile_configuration(
            model=model,
            gateway_id=gateway_id,
            chat_provider_key=chat_provider_key,
            chat_resource_id=chat_resource_id,
            embedding_provider_key=embedding_provider_key,
            embedding_resource_id=embedding_resource_id,
        )
        profile = task_profiles_by_slug.get(model)
        if profile is None:
            created = client.request(
                f"{project_path}/profiles",
                method="POST",
                body={
                    "name": task_profile_name(model),
                    "slug": model,
                    "description": (
                        "StarDojo task route backed by local Vifu Gateway providers."
                    ),
                    **desired,
                    "changeSummary": "Create Stardew Valley task route",
                },
            )["profile"]
            task_profiles_by_slug[model] = created
            created_task_profiles += 1
            continue

        profile_id = str(profile["id"])
        detail = client.request(
            f"{project_path}/profiles/{urllib.parse.quote(profile_id)}"
        )
        if active_profile_matches(detail, desired):
            continue
        created_version = client.request(
            f"{project_path}/profiles/{urllib.parse.quote(profile_id)}/versions",
            method="POST",
            body={
                **desired,
                "changeSummary": "Refresh shared Stardew Valley runtime route",
            },
        )
        version_id = str(created_version["version"]["id"])
        client.request(
            f"{project_path}/profiles/{urllib.parse.quote(profile_id)}"
            f"/versions/{urllib.parse.quote(version_id)}/activate",
            method="POST",
        )
        updated_task_profiles += 1

    model_records = client.request(
        f"/{urllib.parse.quote(project_slug)}/v1/models"
    ).get("data", [])
    available_models = {str(model["id"]) for model in model_records}
    missing_models = [
        model for model in expected_task_models if model not in available_models
    ]
    if missing_models:
        raise RuntimeError(
            "Project did not expose all Stardew Valley task models: "
            + ", ".join(missing_models[:5])
        )
    models = tuple(model for model in expected_task_models if model in available_models)
    return ProjectBootstrap(
        project_id=project_id,
        project_slug=project_slug,
        gateway_id=gateway_id,
        provider_key=chat_provider_key,
        embedding_provider_key=embedding_provider_key,
        models=models,
        physical_agent_count=len(provider_bindings),
        physical_endpoint_count=len(provider_binding_pairs & endpoint_pairs),
        created_endpoints=created_endpoints,
        task_model_count=len(models),
        created_task_profiles=created_task_profiles,
        updated_task_profiles=updated_task_profiles,
    )


def project_key_is_usable(server_url: str, api_key: str) -> bool:
    if not api_key:
        return False
    client = VifuClient(server_url, api_key, authorization_scheme="Bearer")
    try:
        models = client.request("/v1/models").get("data", [])
    except VifuApiError as error:
        if error.status in {401, 403}:
            return False
        raise
    return bool(models)


def project_key_permissions() -> dict[str, str]:
    return {
        "chatCompletions": "access",
        "embeddings": "access",
        "speech": "none",
        "transcriptions": "none",
        "realtime": "none",
        "runtime": "none",
        "agents": "none",
        "project": "none",
    }


def ensure_project_key_permissions(
    client: VifuClient, project_slug: str, raw_key: str
) -> None:
    path = f"/v1/project/{urllib.parse.quote(project_slug)}/api-keys"
    records = client.request(path).get("apiKeys", [])
    prefix = raw_key[:18]
    record = next(
        (
            item
            for item in records
            if item.get("keyPrefix") == prefix and item.get("revokedAt") is None
        ),
        None,
    )
    if record is None:
        raise RuntimeError("existing Project API key could not be identified")
    if record.get("permissions") == project_key_permissions():
        return
    key_id = urllib.parse.quote(str(record["id"]))
    client.request(
        f"{path}/{key_id}",
        method="PATCH",
        body={"permissions": project_key_permissions()},
    )


def verify_project_embeddings(
    server_url: str, project_slug: str, api_key: str, model: str
) -> None:
    client = VifuClient(server_url, api_key, authorization_scheme="Bearer")
    response = client.request(
        f"/{urllib.parse.quote(project_slug)}/v1/embeddings",
        method="POST",
        body={"model": model, "input": "Stardew Valley embedding readiness"},
        timeout=300,
    )
    data = response.get("data", [])
    embedding = data[0].get("embedding", []) if data else []
    if not embedding or not all(isinstance(value, (int, float)) for value in embedding):
        raise RuntimeError("Project embedding endpoint returned an invalid vector")


def bootstrap(args: argparse.Namespace) -> int:
    admin_env_path = Path(args.admin_env).expanduser().resolve()
    admin_env = read_env_file(admin_env_path)
    admin_key = os.getenv("VIFU_ADMIN_KEY") or admin_env.get("VIFU_ADMIN_KEY")
    if not admin_key:
        print(
            f"VIFU_ADMIN_KEY is required in the environment or {admin_env_path}",
            file=sys.stderr,
        )
        return 2
    server_url = (
        args.server_url
        or os.getenv("VIFU_API_BASE_URL")
        or admin_env.get("VIFU_API_BASE_URL")
        or "http://127.0.0.1:6790"
    ).rstrip("/")
    client = VifuClient(server_url, admin_key)
    legacy_provider_key = args.provider_key or "stardew-valley-llama"
    chat_provider_key = args.chat_provider_key or legacy_provider_key
    embedding_provider_key = args.embedding_provider_key or legacy_provider_key
    try:
        configured = ensure_project_configuration(
            client,
            project_slug=args.project_slug,
            project_name=args.project_name,
            chat_provider_key=chat_provider_key,
            embedding_provider_key=embedding_provider_key,
            requested_gateway_id=args.gateway_id,
            task_models=ordered_agents(load_roster()),
        )
        output = Path(args.output).expanduser().resolve()
        existing = read_env_file(output)
        default_model = (
            "stardew-valley-farming-0"
            if "stardew-valley-farming-0" in configured.models
            else configured.models[0]
        )
        project_api_key = existing.get(STARDOJO_API_KEY_ENV, "")
        reused_key = project_key_is_usable(server_url, project_api_key)
        if reused_key:
            ensure_project_key_permissions(
                client, configured.project_slug, project_api_key
            )
        if not reused_key:
            created = client.request(
                f"/v1/project/{urllib.parse.quote(configured.project_slug)}/api-keys",
                method="POST",
                body={
                    "projectId": configured.project_id,
                    "name": "stardew_valley local demo",
                    "agentScope": {"mode": "all"},
                    "permissions": project_key_permissions(),
                },
            )["apiKey"]
            project_api_key = str(created["key"])
            if not project_key_is_usable(server_url, project_api_key):
                raise RuntimeError("new Project API key could not authenticate")
        verify_project_embeddings(
            server_url,
            configured.project_slug,
            project_api_key,
            default_model,
        )
        project_base_url = f"{server_url}/{configured.project_slug}/v1"
        write_private_env(
            output,
            stardojo_env_values(
                existing=existing,
                project_api_key=project_api_key,
                project_base_url=project_base_url,
            ),
        )
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        print(f"bootstrap failed: {error}", file=sys.stderr)
        return 1

    key_status = "reused" if reused_key else "created"
    print(f"Project: {configured.project_slug} ({configured.project_id})")
    print(f"Gateway: {configured.gateway_id}")
    print(f"Chat provider: {configured.provider_key}")
    print(f"Embedding provider: {configured.embedding_provider_key}")
    print(
        "Physical provider Agents/endpoints: "
        f"{configured.physical_agent_count}/{configured.physical_endpoint_count} "
        f"({configured.created_endpoints} endpoints created)"
    )
    print(
        f"Task models: {configured.task_model_count} "
        f"({configured.created_task_profiles} profiles created, "
        f"{configured.updated_task_profiles} updated)"
    )
    print(
        f"Project API key: {key_status}; value written only to "
        f"{output} as {STARDOJO_API_KEY_ENV}"
    )
    print(f"Console: http://localhost:6791/project/{configured.project_slug}")
    print(f"Project API: {project_base_url}")
    return 0


def load_roster() -> dict[str, Any]:
    return json.loads(ROSTER_PATH.read_text(encoding="utf-8"))


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as model:
        while chunk := model.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def git_head(path: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(path), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def health(server_url: str) -> tuple[bool, str]:
    request = urllib.request.Request(f"{server_url.rstrip('/')}/health")
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            payload = json.loads(response.read())
        return payload.get("status") == "ok", f"{payload.get('service')} {payload.get('version')}"
    except (OSError, ValueError, urllib.error.URLError) as error:
        return False, str(error)


def resolve_mod_manifest(smapi: Path | None, explicit: Path | None) -> Path | None:
    if explicit is not None:
        return explicit / "manifest.json" if explicit.is_dir() else explicit
    if smapi is None:
        return None
    return smapi.parent / "Mods" / "StarDojoMod" / "manifest.json"


def configured_model(
    providers_path: Path, provider_key: str | None
) -> tuple[Path | None, str | None]:
    try:
        registry = json.loads(providers_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return None, f"could not read {providers_path}: {error}"
    providers = [
        provider
        for provider in registry.get("providers", [])
        if provider.get("type") == "llama" and provider.get("enabled") is not False
    ]
    if provider_key:
        providers = [provider for provider in providers if provider.get("key") == provider_key]
    if len(providers) != 1:
        detail = "no matching llama provider" if not providers else "multiple llama providers"
        return None, f"{detail} in {providers_path}"
    model_value = providers[0].get("config", {}).get("modelPath")
    if not isinstance(model_value, str) or not model_value.strip():
        return None, "llama provider config.modelPath is required"
    model = Path(model_value).expanduser()
    if not model.is_absolute():
        model = providers_path.parent / model
    return model.resolve(), None


def doctor(args: argparse.Namespace) -> int:
    roster = load_roster()
    checks: list[Check] = []
    machine = platform.machine().lower()
    checks.append(
        Check(
            "Arm host",
            "PASS" if machine in {"arm64", "aarch64"} else "FAIL",
            f"{platform.system()} {machine}",
        )
    )

    stardojo = Path(args.stardojo).expanduser().resolve()
    head = git_head(stardojo)
    expected_head = roster["upstream"]["commit"]
    checks.append(
        Check(
            "StarDojo pin",
            "PASS" if head == expected_head else "FAIL",
            head or "not a Git checkout",
        )
    )
    required_files = [
        "env/llm_env_multi_tasks.py",
        "env/tasks/task_suite/farming_lite.yaml",
        "agent/stardojo/provider/llm/openai.py",
    ]
    missing = [name for name in required_files if not (stardojo / name).is_file()]
    checks.append(
        Check(
            "StarDojo files",
            "PASS" if not missing else "FAIL",
            "present" if not missing else f"missing {', '.join(missing)}",
        )
    )

    smapi_value = args.smapi or os.getenv("STARDEW_APP_PATH")
    smapi = Path(smapi_value).expanduser().resolve() if smapi_value else None
    checks.append(
        Check(
            "SMAPI",
            "PASS" if smapi is not None and smapi.is_file() else "FAIL",
            str(smapi) if smapi is not None else "set STARDEW_APP_PATH or --smapi",
        )
    )
    mod_value = Path(args.mod).expanduser().resolve() if args.mod else None
    mod_manifest = resolve_mod_manifest(smapi, mod_value)
    checks.append(
        Check(
            "StarDojoMod",
            "PASS" if mod_manifest is not None and mod_manifest.is_file() else "FAIL",
            str(mod_manifest) if mod_manifest is not None else "set --mod",
        )
    )

    providers_value = args.providers or os.getenv("VIFU_PROVIDERS_FILE")
    providers_path = (
        Path(providers_value).expanduser().resolve()
        if providers_value
        else Path.home() / ".vifu" / "providers.json"
    )
    provider_key = args.provider_key or os.getenv("VIFU_STARDEW_VALLEY_PROVIDER_KEY")
    model, model_error = configured_model(providers_path, provider_key)
    expected_model = roster["model"]
    if model is None or not model.is_file():
        checks.append(
            Check(
                "Pinned model",
                "FAIL",
                model_error or "configured provider model file is missing",
            )
        )
    else:
        size_ok = model.stat().st_size == expected_model["bytes"]
        digest = sha256(model)
        digest_ok = digest == expected_model["sha256"]
        checks.append(
            Check(
                "Pinned model",
                "PASS" if size_ok and digest_ok else "FAIL",
                f"{model.name}, {model.stat().st_size} bytes, sha256={digest}",
            )
        )

    server_ok, server_detail = health(args.server_url)
    checks.append(Check("Vifu Server", "PASS" if server_ok else "FAIL", server_detail))

    for check in checks:
        print(f"[{check.status}] {check.name}: {check.detail}")
    failed = sum(check.status == "FAIL" for check in checks)
    print(f"\n{len(checks) - failed}/{len(checks)} checks passed")
    return 1 if failed else 0


def ordered_agents(roster: dict[str, Any]) -> list[str]:
    counts = {item["name"]: item["tasks"] for item in roster["categories"]}
    demo = [
        f"stardew-valley-{task['suite'].removesuffix('_lite')}-{task['taskId']}"
        for task in roster["demoBoard"]
    ]
    all_agents = [
        f"stardew-valley-{category['name']}-{task_id}"
        for category in roster["categories"]
        for task_id in range(category["tasks"])
    ]
    farming = [
        f"stardew-valley-farming-{task_id}" for task_id in range(counts["farming"])
    ]
    ordered: list[str] = []
    for agent in demo + farming + all_agents:
        if agent not in ordered:
            ordered.append(agent)
    return ordered


def invoke_smoke(base_url: str, api_key: str, agent: str) -> dict[str, Any]:
    schema = {
        "type": "object",
        "properties": {"ok": {"type": "boolean", "const": True}},
        "required": ["ok"],
        "additionalProperties": False,
    }
    body = json.dumps(
        {
            "model": agent,
            "messages": [{"role": "user", "content": "Return ok=true."}],
            "max_tokens": 16,
            "temperature": 0,
            "seed": 0,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": "vifu_smoke", "strict": True, "schema": schema},
            },
        }
    ).encode()
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/chat/completions",
        data=body,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=180) as response:
        payload = json.loads(response.read())
    elapsed_ms = round((time.perf_counter() - started) * 1000, 2)
    content = payload["choices"][0]["message"]["content"]
    structured = json.loads(content)
    if structured != {"ok": True}:
        raise ValueError("smoke response did not satisfy the schema")
    usage = payload.get("usage", {})
    return {
        "agent": agent,
        "ok": True,
        "latencyMs": elapsed_ms,
        "promptTokens": usage.get("prompt_tokens"),
        "completionTokens": usage.get("completion_tokens"),
    }


def smoke(args: argparse.Namespace) -> int:
    roster = load_roster()
    if args.stage not in roster["rosterStages"]:
        print(f"stage must be one of {roster['rosterStages']}", file=sys.stderr)
        return 2
    api_key = os.getenv(args.api_key_env)
    if not api_key:
        print(f"{args.api_key_env} is required", file=sys.stderr)
        return 2
    base_url = args.base_url or os.getenv(OPENAI_BASE_URL_ENV)
    if not base_url:
        print(f"--base-url or {OPENAI_BASE_URL_ENV} is required", file=sys.stderr)
        return 2
    agents = ordered_agents(roster)[: args.stage]
    output = Path(args.output or f"target/stardew_valley/roster-{args.stage}.jsonl")
    output.parent.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    with output.open("w", encoding="utf-8") as stream:
        for index, agent in enumerate(agents, start=1):
            try:
                result = invoke_smoke(base_url, api_key, agent)
                print(f"[{index}/{len(agents)}] PASS {agent} {result['latencyMs']}ms")
            except (KeyError, ValueError, OSError, urllib.error.HTTPError) as error:
                result = {"agent": agent, "ok": False, "error": str(error)}
                print(f"[{index}/{len(agents)}] FAIL {agent}: {error}")
            results.append(result)
            stream.write(json.dumps(result, separators=(",", ":")) + "\n")
            stream.flush()
    passed = sum(result["ok"] for result in results)
    print(f"\n{passed}/{len(results)} agents completed a real structured invocation")
    print(f"Results: {output}")
    return 0 if passed == len(results) else 1


def identity(args: argparse.Namespace) -> int:
    output = Path(args.output).expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(output, flags, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(f"VIFU_GATEWAY_ID=gateway-{uuid.uuid4().hex}\n")
            stream.write(f"VIFU_GATEWAY_CREDENTIAL=vifu_gw_{secrets.token_hex(32)}\n")
    except BaseException:
        output.unlink(missing_ok=True)
        raise
    print(f"Gateway identity written with private permissions: {output}")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    doctor_parser = commands.add_parser("doctor", help="check the complete local demo chain")
    doctor_parser.add_argument("--stardojo", required=True)
    doctor_parser.add_argument("--providers")
    doctor_parser.add_argument("--provider-key")
    doctor_parser.add_argument("--smapi")
    doctor_parser.add_argument("--mod")
    doctor_parser.add_argument("--server-url", default="http://127.0.0.1:6790")
    doctor_parser.set_defaults(run=doctor)

    bootstrap_parser = commands.add_parser(
        "bootstrap", help="create or update the local Stardew Valley Project configuration"
    )
    bootstrap_parser.add_argument("--admin-env", default=str(DEFAULT_ADMIN_ENV))
    bootstrap_parser.add_argument("--server-url")
    bootstrap_parser.add_argument("--project-slug", default="stardew-valley")
    bootstrap_parser.add_argument("--project-name", default="stardew_valley")
    bootstrap_parser.add_argument("--provider-key")
    bootstrap_parser.add_argument("--chat-provider-key")
    bootstrap_parser.add_argument("--embedding-provider-key")
    bootstrap_parser.add_argument("--gateway-id")
    bootstrap_parser.add_argument("--output", default=str(DEFAULT_DEMO_ENV))
    bootstrap_parser.set_defaults(run=bootstrap)

    smoke_parser = commands.add_parser("smoke", help="invoke a staged set of real task agents")
    smoke_parser.add_argument("--stage", type=int, default=5)
    smoke_parser.add_argument("--base-url")
    smoke_parser.add_argument("--api-key-env", default=STARDOJO_API_KEY_ENV)
    smoke_parser.add_argument("--output")
    smoke_parser.set_defaults(run=smoke)

    identity_parser = commands.add_parser(
        "identity", help="create a private, sourceable gateway identity file"
    )
    identity_parser.add_argument("--output", default=".vifu-stardew-valley-identity")
    identity_parser.set_defaults(run=identity)
    return root


def main() -> int:
    args = parser().parse_args()
    return args.run(args)


if __name__ == "__main__":
    raise SystemExit(main())
