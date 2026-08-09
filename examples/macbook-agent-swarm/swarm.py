#!/usr/bin/env python3
"""Run a repeatable multi-agent benchmark through a local Vifu Gateway.

Start `vifu` in one terminal, then run this script from another terminal that
has the same VIFU_ADMIN_KEY. The script configures only its own project and
never reads or changes ~/.vifu/providers.json.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import html
import json
import math
import os
import platform
import statistics
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable


DEFAULT_SERVER_URL = "http://127.0.0.1:6790"
DEFAULT_PROJECT_SLUG = "macbook-agent-swarm"
PROFILE_TIMEOUT_MS = 120_000


class VifuApiError(RuntimeError):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(f"Vifu returned {status} ({code}): {message}")
        self.status = status
        self.code = code
        self.message = message


class VifuClient:
    def __init__(self, server_url: str, credential: str, scheme: str = "Vifu") -> None:
        self.server_url = server_url.rstrip("/")
        self.credential = credential
        self.scheme = scheme

    def request(
        self,
        path: str,
        *,
        method: str = "GET",
        body: dict[str, Any] | None = None,
        timeout_seconds: float | None = None,
    ) -> tuple[dict[str, Any], dict[str, str]]:
        data = json.dumps(body, separators=(",", ":")).encode("utf-8") if body else None
        headers = {
            "Accept": "application/json",
            "Authorization": f"{self.scheme} {self.credential}",
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
            if timeout_seconds is None:
                response = urllib.request.urlopen(request)
            else:
                response = urllib.request.urlopen(request, timeout=timeout_seconds)
            with response:
                raw = response.read()
                response_headers = dict(response.headers.items())
        except urllib.error.HTTPError as error:
            payload = decode_json(error.read())
            details = payload.get("error", {}) if isinstance(payload, dict) else {}
            raise VifuApiError(
                error.code,
                str(details.get("code", "unknown")),
                safe_text(str(details.get("message", "request failed"))),
            ) from error
        except urllib.error.URLError as error:
            raise RuntimeError(f"could not reach Vifu at {self.server_url}: {error.reason}") from error
        return decode_json(raw), response_headers


@dataclass(frozen=True)
class ProviderResource:
    gateway_id: str
    resource_id: str


@dataclass(frozen=True)
class ProjectRoute:
    project_id: str
    project_slug: str
    models: tuple[str, ...]


@dataclass(frozen=True)
class InvocationResult:
    agent: str
    sequence: int
    latency_ms: int
    status: str
    request_id: str | None
    prompt_tokens: int | None
    completion_tokens: int | None
    error: str | None


def decode_json(raw: bytes) -> dict[str, Any]:
    if not raw:
        return {}
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise RuntimeError("Vifu returned invalid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError("Vifu returned an unexpected JSON value")
    return value


def safe_text(value: str, limit: int = 300) -> str:
    normalized = " ".join(value.split())
    return normalized[:limit]


def provider_agents(gateway: dict[str, Any], provider_key: str) -> list[dict[str, Any]]:
    return [
        agent
        for agent in gateway.get("agents", [])
        if agent.get("status") in {None, "connected", "online"}
        and agent.get("metadata", {}).get("providerKey") == provider_key
    ]


def select_provider_resource(
    gateways: Iterable[dict[str, Any]], provider_key: str
) -> ProviderResource:
    candidates = [
        (gateway, provider_agents(gateway, provider_key))
        for gateway in gateways
        if gateway.get("status") == "connected"
    ]
    candidates = [(gateway, agents) for gateway, agents in candidates if agents]
    if not candidates:
        raise RuntimeError(
            f"no connected Gateway advertises provider {provider_key!r}; "
            "start vifu and confirm ~/.vifu/providers.json contains that provider"
        )
    if len(candidates) > 1:
        identifiers = ", ".join(str(gateway.get("gatewayId")) for gateway, _ in candidates)
        raise RuntimeError(
            f"provider {provider_key!r} is available from multiple Gateways ({identifiers}); "
            "use a Vifu project that selects one Gateway before running this demo"
        )
    gateway, agents = candidates[0]
    exact = [agent for agent in agents if str(agent.get("id")) == provider_key]
    if len(exact) == 1:
        resource = exact[0]
    elif len(agents) == 1:
        resource = agents[0]
    else:
        identifiers = ", ".join(str(agent.get("id")) for agent in agents)
        raise RuntimeError(
            f"provider {provider_key!r} advertises multiple resources ({identifiers}); "
            "give the provider one chat resource for this benchmark"
        )
    return ProviderResource(str(gateway["gatewayId"]), str(resource["id"]))


def wait_for_provider(
    client: VifuClient, provider_key: str, wait_seconds: float
) -> ProviderResource:
    deadline = None if wait_seconds == 0 else time.monotonic() + wait_seconds
    last_error: RuntimeError | None = None
    while True:
        try:
            gateways, _ = client.request("/v1/agent-gateways")
            return select_provider_resource(gateways.get("agentGateways", []), provider_key)
        except VifuApiError:
            raise
        except RuntimeError as error:
            last_error = error
        if deadline is not None and time.monotonic() >= deadline:
            raise RuntimeError(
                f"provider {provider_key!r} was not ready after {wait_seconds:g} seconds"
            ) from last_error
        time.sleep(1)


def route_configuration(
    *,
    agent_slug: str,
    resource: ProviderResource,
    provider_key: str,
) -> dict[str, Any]:
    source = {
        "type": "vifu-runtime",
        "managed": True,
        "gatewayId": resource.gateway_id,
        "providerKey": provider_key,
        "resourceId": resource.resource_id,
        "integration": "macbook-agent-swarm",
    }
    return {
        "persona": {
            "files": {
                "benchmark.md": (
                    "You are one logical benchmark agent. Follow the requested output "
                    "format exactly and do not add commentary."
                )
            }
        },
        "runtime": {"requestTimeoutMs": PROFILE_TIMEOUT_MS},
        "presentation": {"demo": "macbook-agent-swarm", "agent": agent_slug},
        "source": source,
        "capabilities": [
            {
                "kind": "chat",
                "providerType": "vifu-runtime",
                "providerKey": provider_key,
                "resourceId": resource.resource_id,
                "config": {
                    "gatewayId": resource.gateway_id,
                    "source": "macbook-agent-swarm",
                },
                "inputSchema": {},
                "outputSchema": {},
            }
        ],
    }


def active_profile_matches(detail: dict[str, Any], desired: dict[str, Any]) -> bool:
    active_id = str(detail.get("profile", {}).get("activeVersionId", ""))
    active = next(
        (
            item
            for item in detail.get("versions", [])
            if str(item.get("version", {}).get("id", "")) == active_id
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
    fields = (
        "kind",
        "providerType",
        "providerKey",
        "resourceId",
        "config",
        "inputSchema",
        "outputSchema",
    )
    capabilities = [
        {field: capability.get(field) for field in fields}
        for capability in active.get("capabilities", [])
    ]
    return capabilities == desired["capabilities"]


def ensure_project_provider(
    client: VifuClient, project_slug: str, provider_key: str
) -> None:
    path = f"/v1/project/{urllib.parse.quote(project_slug)}/providers"
    providers, _ = client.request(path)
    assigned = any(
        provider.get("sourceKind") == "custom" and provider.get("sourceKey") == provider_key
        for provider in providers.get("providers", [])
    )
    if not assigned:
        client.request(
            path,
            method="POST",
            body={"source": {"kind": "custom", "key": provider_key}},
        )


def ensure_primary_deployment(client: VifuClient, project_slug: str) -> None:
    project_path = f"/v1/project/{urllib.parse.quote(project_slug)}"
    response, _ = client.request(f"{project_path}/deployments")
    deployment = next(
        (item for item in response.get("deployments", []) if item.get("isPrimary") is True),
        None,
    )
    if deployment is None:
        raise RuntimeError("benchmark project did not receive a primary runtime deployment")
    if deployment.get("remoteInvocationEnabled") is not True:
        client.request(
            f"{project_path}/deployments/{urllib.parse.quote(str(deployment['name']))}",
            method="PATCH",
            body={"remoteInvocationEnabled": True},
        )


def ensure_profile(
    client: VifuClient,
    *,
    project_slug: str,
    profile: dict[str, Any] | None,
    name: str,
    slug: str,
    desired: dict[str, Any],
) -> None:
    path = f"/v1/project/{urllib.parse.quote(project_slug)}/profiles"
    if profile is None:
        client.request(
            path,
            method="POST",
            body={
                "name": name,
                "slug": slug,
                "description": "Logical agent route for the macOS ARM swarm benchmark.",
                **desired,
                "changeSummary": "Create MacBook swarm benchmark route",
            },
        )
        return
    profile_id = str(profile["id"])
    detail, _ = client.request(f"{path}/{urllib.parse.quote(profile_id)}")
    if active_profile_matches(detail, desired):
        return
    created, _ = client.request(
        f"{path}/{urllib.parse.quote(profile_id)}/versions",
        method="POST",
        body={**desired, "changeSummary": "Refresh MacBook swarm benchmark route"},
    )
    version_id = str(created["version"]["id"])
    client.request(
        f"{path}/{urllib.parse.quote(profile_id)}/versions/{urllib.parse.quote(version_id)}/activate",
        method="POST",
    )


def configure_project(
    client: VifuClient,
    *,
    project_slug: str,
    provider_key: str,
    agents: int,
    wait_seconds: float,
) -> ProjectRoute:
    resource = wait_for_provider(client, provider_key, wait_seconds)
    projects, _ = client.request("/v1/projects")
    project = next(
        (item for item in projects.get("projects", []) if item.get("slug") == project_slug),
        None,
    )
    if project is None:
        created, _ = client.request(
            "/v1/projects",
            method="POST",
            body={"name": "MacBook agent swarm", "slug": project_slug},
        )
        project = created["project"]
    updated, _ = client.request(
        f"/v1/projects/{urllib.parse.quote(str(project['id']))}",
        method="PATCH",
        body={"gatewayId": resource.gateway_id},
    )
    project = updated["project"]
    project_slug = str(project["slug"])
    ensure_primary_deployment(client, project_slug)
    ensure_project_provider(client, project_slug, provider_key)

    profiles_response, _ = client.request(
        f"/v1/project/{urllib.parse.quote(project_slug)}/profiles"
    )
    existing = {str(profile.get("slug")): profile for profile in profiles_response.get("profiles", [])}
    models: list[str] = []
    for number in range(1, agents + 1):
        slug = f"macbook-swarm-agent-{number:02d}"
        desired = route_configuration(
            agent_slug=slug, resource=resource, provider_key=provider_key
        )
        ensure_profile(
            client,
            project_slug=project_slug,
            profile=existing.get(slug),
            name=f"MacBook swarm agent {number:02d}",
            slug=slug,
            desired=desired,
        )
        models.append(slug)
    return ProjectRoute(str(project["id"]), project_slug, tuple(models))


def project_key_permissions() -> dict[str, str]:
    return {
        "chatCompletions": "access",
        "embeddings": "none",
        "speech": "none",
        "transcriptions": "none",
        "realtime": "none",
        "runtime": "none",
        "agents": "none",
        "project": "none",
    }


def create_ephemeral_project_key(
    client: VifuClient, route: ProjectRoute
) -> tuple[str, str]:
    created, _ = client.request(
        f"/v1/project/{urllib.parse.quote(route.project_slug)}/api-keys",
        method="POST",
        body={
            "projectId": route.project_id,
            "name": "macbook-agent-swarm temporary benchmark key",
            "agentScope": {"mode": "all"},
            "permissions": project_key_permissions(),
        },
    )
    key = created["apiKey"]
    return str(key["id"]), str(key["key"])


def delete_project_key(client: VifuClient, project_slug: str, key_id: str) -> None:
    path = (
        f"/v1/project/{urllib.parse.quote(project_slug)}/api-keys/"
        f"{urllib.parse.quote(key_id)}"
    )
    client.request(f"{path}/revoke", method="POST")
    client.request(path, method="DELETE")


def chat_request(
    client: VifuClient,
    *,
    model: str,
    sequence: int,
    max_tokens: int,
    timeout_seconds: float | None,
) -> InvocationResult:
    started = time.perf_counter()
    try:
        response, headers = client.request(
            "/v1/chat/completions",
            method="POST",
            body={
                "model": model,
                "max_tokens": max_tokens,
                "temperature": 0,
                "user": model,
                "messages": [
                    {
                        "role": "user",
                        "content": f"Reply exactly with: {model} ready {sequence}",
                    }
                ],
            },
            timeout_seconds=timeout_seconds,
        )
        usage = response.get("usage", {})
        return InvocationResult(
            agent=model,
            sequence=sequence,
            latency_ms=round((time.perf_counter() - started) * 1000),
            status="ok",
            request_id=headers.get("x-request-id"),
            prompt_tokens=integer_or_none(usage.get("prompt_tokens")),
            completion_tokens=integer_or_none(usage.get("completion_tokens")),
            error=None,
        )
    except (RuntimeError, VifuApiError) as error:
        return InvocationResult(
            agent=model,
            sequence=sequence,
            latency_ms=round((time.perf_counter() - started) * 1000),
            status="error",
            request_id=None,
            prompt_tokens=None,
            completion_tokens=None,
            error=safe_text(str(error)),
        )


def integer_or_none(value: Any) -> int | None:
    return value if isinstance(value, int) and value >= 0 else None


def run_agent(
    client: VifuClient,
    route: ProjectRoute,
    model: str,
    requests_per_agent: int,
    max_tokens: int,
    timeout_seconds: float | None,
) -> list[InvocationResult]:
    return [
        chat_request(
            client,
            model=model,
            sequence=sequence,
            max_tokens=max_tokens,
            timeout_seconds=timeout_seconds,
        )
        for sequence in range(1, requests_per_agent + 1)
    ]


def run_benchmark(
    client: VifuClient,
    route: ProjectRoute,
    *,
    requests_per_agent: int,
    max_in_flight: int,
    warmups: int,
    max_tokens: int,
    timeout_seconds: float | None,
) -> tuple[list[InvocationResult], int]:
    for sequence in range(1, warmups + 1):
        warmup = chat_request(
            client,
            model=route.models[0],
            sequence=-sequence,
            max_tokens=max_tokens,
            timeout_seconds=timeout_seconds,
        )
        if warmup.status != "ok":
            raise RuntimeError(f"warmup failed: {warmup.error}")

    started = time.perf_counter()
    results: list[InvocationResult] = []
    result_lock = threading.Lock()
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=min(max_in_flight, len(route.models)), thread_name_prefix="vifu-swarm"
    ) as executor:
        futures = [
            executor.submit(
                run_agent,
                client,
                route,
                model,
                requests_per_agent,
                max_tokens,
                timeout_seconds,
            )
            for model in route.models
        ]
        for future in concurrent.futures.as_completed(futures):
            agent_results = future.result()
            with result_lock:
                results.extend(agent_results)
    elapsed_ms = round((time.perf_counter() - started) * 1000)
    results.sort(key=lambda item: (item.agent, item.sequence))
    return results, elapsed_ms


def percentile(values: list[int], fraction: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * fraction) - 1)
    return ordered[index]


def benchmark_summary(results: list[InvocationResult], elapsed_ms: int) -> dict[str, Any]:
    successes = [result for result in results if result.status == "ok"]
    latencies = [result.latency_ms for result in successes]
    completion_tokens = sum(result.completion_tokens or 0 for result in successes)
    return {
        "requests": len(results),
        "succeeded": len(successes),
        "failed": len(results) - len(successes),
        "wallTimeMs": elapsed_ms,
        "latencyMs": {
            "min": min(latencies) if latencies else None,
            "mean": round(statistics.fmean(latencies), 1) if latencies else None,
            "p50": percentile(latencies, 0.50),
            "p95": percentile(latencies, 0.95),
            "p99": percentile(latencies, 0.99),
            "max": max(latencies) if latencies else None,
        },
        "completionTokens": completion_tokens,
        "completionTokensPerSecond": (
            round(completion_tokens / (elapsed_ms / 1000), 2) if elapsed_ms > 0 else None
        ),
    }


def default_report_path() -> Path:
    if platform.system() == "Darwin":
        return Path.home() / "Library" / "Caches" / "Vifu" / "macbook-agent-swarm.json"
    return Path.home() / ".cache" / "vifu" / "macbook-agent-swarm.json"


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def default_chart_path(report_path: Path) -> Path:
    return report_path.with_suffix(".html")


def chart_number(value: Any, *, suffix: str = "", digits: int = 0) -> str:
    if isinstance(value, (int, float)):
        return f"{value:.{digits}f}{suffix}"
    return "—"


def render_report_chart(report: dict[str, Any]) -> str:
    """Render a local, dependency-free HTML/SVG view of a safe JSON report."""
    summary = report.get("summary", {})
    latency = summary.get("latencyMs", {}) if isinstance(summary, dict) else {}
    results = report.get("results", [])
    if not isinstance(results, list):
        results = []

    safe_results = [item for item in results if isinstance(item, dict)]
    max_latency = max(
        (item.get("latency_ms", 0) for item in safe_results if isinstance(item.get("latency_ms"), int)),
        default=1,
    )
    chart_width = 960
    label_width = 220
    right_padding = 92
    plot_width = chart_width - label_width - right_padding
    row_height = 32
    chart_height = max(112, 50 + len(safe_results) * row_height)
    rows: list[str] = []
    for index, result in enumerate(safe_results):
        y = 28 + index * row_height
        latency_ms = result.get("latency_ms")
        latency_value = latency_ms if isinstance(latency_ms, int) and latency_ms >= 0 else 0
        status = str(result.get("status", "unknown"))
        color = "#33b074" if status == "ok" else "#e15858"
        bar_width = round(plot_width * latency_value / max_latency, 1)
        label = f"{result.get('agent', 'unknown')} #{result.get('sequence', '?')}"
        error = result.get("error")
        detail = f"{latency_value} ms · {status}"
        if error:
            detail = f"{detail} · {safe_text(str(error), 120)}"
        rows.append(
            "\n".join(
                [
                    f'<text class="label" x="0" y="{y + 15}">{html.escape(label)}</text>',
                    f'<rect class="track" x="{label_width}" y="{y}" width="{plot_width}" height="18" rx="5" />',
                    f'<rect class="bar" x="{label_width}" y="{y}" width="{bar_width}" height="18" rx="5" fill="{color}"><title>{html.escape(detail)}</title></rect>',
                    f'<text class="value" x="{label_width + plot_width + 12}" y="{y + 15}">{latency_value} ms</text>',
                ]
            )
        )
    if not rows:
        rows.append('<text class="empty" x="0" y="42">No measured requests were recorded.</text>')

    cards = [
        ("Requests", f"{summary.get('succeeded', 0)}/{summary.get('requests', 0)}", "succeeded"),
        ("Wall time", chart_number(summary.get("wallTimeMs"), suffix=" ms"), "all measured requests"),
        ("p95 latency", chart_number(latency.get("p95"), suffix=" ms"), "successful requests"),
        (
            "Completion throughput",
            chart_number(summary.get("completionTokensPerSecond"), suffix=" tokens/s", digits=2),
            "aggregate",
        ),
    ]
    card_html = "\n".join(
        (
            '<section class="metric">'
            f'<span>{html.escape(label)}</span><strong>{html.escape(value)}</strong>'
            f'<small>{html.escape(detail)}</small></section>'
        )
        for label, value, detail in cards
    )
    project = html.escape(str(report.get("project", "unknown project")))
    provider = html.escape(str(report.get("providerKey", "unknown provider")))
    agents = html.escape(str(report.get("agents", "?")))
    max_in_flight = html.escape(str(report.get("maxInFlight", "?")))
    requests_per_agent = html.escape(str(report.get("requestsPerAgent", "?")))
    axis_max = html.escape(chart_number(max_latency, suffix=" ms"))

    return f"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Vifu MacBook swarm report</title>
<style>
  :root {{ color-scheme: dark; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
  body {{ margin: 0; background: #0b1020; color: #eff5ff; }}
  main {{ max-width: 1040px; margin: 0 auto; padding: 40px 24px 56px; }}
  h1 {{ margin: 0; font: 700 30px/1.2 system-ui, sans-serif; }}
  h2 {{ margin: 38px 0 14px; font: 700 17px/1.3 system-ui, sans-serif; }}
  .subtitle, .metadata {{ color: #9fb0ca; line-height: 1.6; }}
  .metadata {{ font-size: 13px; }}
  .metrics {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 12px; margin-top: 24px; }}
  .metric, .chart {{ border: 1px solid #283653; background: #121b31; border-radius: 12px; }}
  .metric {{ padding: 16px; }}
  .metric span, .metric small {{ display: block; color: #9fb0ca; font-size: 12px; }}
  .metric strong {{ display: block; margin: 8px 0; color: #f7fbff; font-size: 21px; }}
  .chart {{ overflow-x: auto; padding: 22px; }}
  svg {{ display: block; min-width: 700px; width: 100%; }}
  .label {{ fill: #d9e7fb; font-size: 12px; }}
  .value {{ fill: #9fb0ca; font-size: 12px; }}
  .track {{ fill: #202d48; }}
  .empty {{ fill: #9fb0ca; font-size: 13px; }}
  .scale {{ display: flex; justify-content: space-between; color: #7385a4; font-size: 11px; margin: 0 92px 8px 220px; }}
  footer {{ margin-top: 30px; color: #7385a4; font-size: 12px; line-height: 1.6; }}
</style>
<main>
  <h1>MacBook agent swarm</h1>
  <p class="subtitle">Project <strong>{project}</strong> · Provider <strong>{provider}</strong></p>
  <p class="metadata">{agents} logical agents · {requests_per_agent} request(s) per agent · {max_in_flight} max in flight</p>
  <div class="metrics">{card_html}</div>
  <h2>Latency by request</h2>
  <div class="chart">
    <div class="scale"><span>0 ms</span><span>{axis_max}</span></div>
    <svg viewBox="0 0 {chart_width} {chart_height}" role="img" aria-label="Latency by benchmark request">
      {''.join(rows)}
    </svg>
  </div>
  <footer>Generated locally by Vifu. This report intentionally excludes prompts, model completions, and credentials.</footer>
</main>
</html>
"""


def write_chart(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(f"{path.suffix}.tmp")
    temporary.write_text(render_report_chart(report), encoding="utf-8")
    temporary.replace(path)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider-key", required=True, help="Provider key from ~/.vifu/providers.json")
    parser.add_argument("--project-slug", default=DEFAULT_PROJECT_SLUG)
    parser.add_argument("--agents", type=int, default=8, help="Logical agents to configure (default: 8)")
    parser.add_argument(
        "--requests-per-agent", type=int, default=3, help="Sequential requests per agent (default: 3)"
    )
    parser.add_argument("--max-in-flight", type=int, default=8, help="Concurrent logical agents (default: 8)")
    parser.add_argument("--warmups", type=int, default=1, help="Warmup requests before measuring (default: 1)")
    parser.add_argument("--max-tokens", type=int, default=32, help="Completion token limit (default: 32)")
    parser.add_argument(
        "--setup-wait-seconds",
        type=float,
        default=60,
        help="Wait for the Vifu Gateway to advertise the Provider; 0 waits indefinitely (default: 60)",
    )
    parser.add_argument(
        "--request-timeout-seconds",
        type=float,
        default=0,
        help="Optional Python client deadline; 0 leaves timing to Vifu's route timeout (default: 0)",
    )
    parser.add_argument(
        "--server-url",
        default=os.environ.get("VIFU_API_BASE_URL", DEFAULT_SERVER_URL),
        help=f"Vifu Server URL (default: {DEFAULT_SERVER_URL})",
    )
    parser.add_argument("--report", type=Path, default=default_report_path())
    parser.add_argument(
        "--chart",
        type=Path,
        help="Optional HTML chart path (default: report path with a .html suffix)",
    )
    parser.add_argument(
        "--no-chart",
        action="store_true",
        help="Write only the JSON report",
    )
    args = parser.parse_args()
    for name in ("agents", "requests_per_agent", "max_in_flight", "max_tokens"):
        if getattr(args, name) < 1:
            parser.error(f"--{name.replace('_', '-')} must be at least 1")
    if args.warmups < 0 or args.setup_wait_seconds < 0 or args.request_timeout_seconds < 0:
        parser.error("wait and timeout values cannot be negative")
    return args


def main() -> int:
    args = parse_arguments()
    admin_key = os.environ.get("VIFU_ADMIN_KEY", "").strip()
    if not admin_key:
        print("VIFU_ADMIN_KEY must be present for project setup.", file=sys.stderr)
        return 2
    if platform.system() != "Darwin" or platform.machine().lower() not in {"arm64", "aarch64"}:
        print(
            "Warning: this demo is designed to compare Apple-silicon MacBook runs; continuing.",
            file=sys.stderr,
        )
    admin = VifuClient(args.server_url, admin_key)
    try:
        route = configure_project(
            admin,
            project_slug=args.project_slug,
            provider_key=args.provider_key,
            agents=args.agents,
            wait_seconds=args.setup_wait_seconds,
        )
        key_id, project_key = create_ephemeral_project_key(admin, route)
    except (RuntimeError, VifuApiError) as error:
        print(f"Setup failed: {safe_text(str(error))}", file=sys.stderr)
        return 1

    key_client = VifuClient(args.server_url, project_key, scheme="Bearer")
    timeout_seconds = args.request_timeout_seconds or None
    try:
        results, elapsed_ms = run_benchmark(
            key_client,
            route,
            requests_per_agent=args.requests_per_agent,
            max_in_flight=args.max_in_flight,
            warmups=args.warmups,
            max_tokens=args.max_tokens,
            timeout_seconds=timeout_seconds,
        )
        summary = benchmark_summary(results, elapsed_ms)
        report = {
            "schemaVersion": 1,
            "device": {
                "system": platform.system(),
                "release": platform.release(),
                "machine": platform.machine(),
                "processor": platform.processor(),
                "cpuCount": os.cpu_count(),
            },
            "project": route.project_slug,
            "providerKey": args.provider_key,
            "agents": len(route.models),
            "requestsPerAgent": args.requests_per_agent,
            "maxInFlight": args.max_in_flight,
            "warmups": args.warmups,
            "maxTokens": args.max_tokens,
            "routeTimeoutMs": PROFILE_TIMEOUT_MS,
            "clientTimeoutSeconds": args.request_timeout_seconds or None,
            "summary": summary,
            "results": [result.__dict__ for result in results],
        }
        report_path = args.report.expanduser()
        write_report(report_path, report)
        chart_path = None
        if not args.no_chart:
            chart_path = (args.chart or default_chart_path(report_path)).expanduser()
            write_chart(chart_path, report)
    except (RuntimeError, VifuApiError) as error:
        print(f"Benchmark failed: {safe_text(str(error))}", file=sys.stderr)
        return_code = 1
    else:
        print(
            "Swarm complete: "
            f"{summary['succeeded']}/{summary['requests']} requests succeeded; "
            f"wall {summary['wallTimeMs']} ms; "
            f"p95 {summary['latencyMs']['p95']} ms; "
            f"completion throughput {summary['completionTokensPerSecond']} tokens/s."
        )
        print(f"Report: {report_path}")
        if chart_path is not None:
            print(f"Chart: {chart_path}")
        return_code = 0 if summary["failed"] == 0 else 1
    finally:
        try:
            delete_project_key(admin, route.project_slug, key_id)
        except (RuntimeError, VifuApiError) as error:
            print(f"Could not remove the temporary project key: {safe_text(str(error))}", file=sys.stderr)
            return_code = 1
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
