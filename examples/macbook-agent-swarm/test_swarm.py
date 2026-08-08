#!/usr/bin/env python3
"""Focused regression tests for the MacBook swarm benchmark helpers."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("swarm.py")
SPEC = importlib.util.spec_from_file_location("macbook_agent_swarm", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
swarm = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = swarm
SPEC.loader.exec_module(swarm)


class MacbookAgentSwarmTests(unittest.TestCase):
    def test_temporary_key_is_revoked_before_deletion(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.calls: list[tuple[str, str]] = []

            def request(self, path: str, *, method: str = "GET") -> tuple[dict[str, object], dict[str, str]]:
                self.calls.append((method, path))
                return ({}, {})

        client = FakeClient()
        swarm.delete_project_key(client, "macbook-agent-swarm", "key-id")

        self.assertEqual(client.calls, [
            ("POST", "/v1/project/macbook-agent-swarm/api-keys/key-id/revoke"),
            ("DELETE", "/v1/project/macbook-agent-swarm/api-keys/key-id"),
        ])

    def test_configure_project_creates_isolated_logical_routes(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.created_profiles: list[dict[str, object]] = []
                self.calls: list[tuple[str, str, dict[str, object] | None]] = []

            def request(
                self,
                path: str,
                *,
                method: str = "GET",
                body: dict[str, object] | None = None,
                timeout_seconds: float | None = None,
            ) -> tuple[dict[str, object], dict[str, str]]:
                del timeout_seconds
                self.calls.append((method, path, body))
                if path == "/v1/agent-gateways":
                    return ({
                        "agentGateways": [{
                            "gatewayId": "gateway-local",
                            "status": "connected",
                            "agents": [{
                                "id": "local-qwen",
                                "metadata": {"providerKey": "local-qwen"},
                            }],
                        }]
                    }, {})
                if path == "/v1/projects" and method == "GET":
                    return ({"projects": []}, {})
                if path == "/v1/projects" and method == "POST":
                    return ({"project": {"id": "project-id", "slug": "macbook-agent-swarm"}}, {})
                if path == "/v1/projects/project-id" and method == "PATCH":
                    return ({"project": {"id": "project-id", "slug": "macbook-agent-swarm"}}, {})
                if path == "/v1/project/macbook-agent-swarm/deployments" and method == "GET":
                    return ({"deployments": [{"name": "development", "isPrimary": True, "remoteInvocationEnabled": False}]}, {})
                if path == "/v1/project/macbook-agent-swarm/deployments/development" and method == "PATCH":
                    return ({"deployment": {}}, {})
                if path == "/v1/project/macbook-agent-swarm/providers" and method == "GET":
                    return ({"providers": []}, {})
                if path == "/v1/project/macbook-agent-swarm/providers" and method == "POST":
                    return ({"provider": {}}, {})
                if path == "/v1/project/macbook-agent-swarm/profiles" and method == "GET":
                    return ({"profiles": []}, {})
                if path == "/v1/project/macbook-agent-swarm/profiles" and method == "POST":
                    assert body is not None
                    self.created_profiles.append(body)
                    return ({"profile": {"id": f"profile-{len(self.created_profiles)}"}}, {})
                raise AssertionError(f"unexpected request: {method} {path}")

        client = FakeClient()
        route = swarm.configure_project(
            client,
            project_slug="macbook-agent-swarm",
            provider_key="local-qwen",
            agents=2,
            wait_seconds=1,
        )

        self.assertEqual(route.project_id, "project-id")
        self.assertEqual(route.models, ("macbook-swarm-agent-01", "macbook-swarm-agent-02"))
        self.assertEqual(len(client.created_profiles), 2)
        source = client.created_profiles[0]["source"]
        self.assertIsInstance(source, dict)
        self.assertEqual(source["resourceId"], "local-qwen")

    def test_select_provider_resource_prefers_matching_resource_id(self) -> None:
        resource = swarm.select_provider_resource(
            [
                {
                    "gatewayId": "gateway-local",
                    "status": "connected",
                    "agents": [
                        {"id": "secondary", "metadata": {"providerKey": "local-qwen"}},
                        {"id": "local-qwen", "metadata": {"providerKey": "local-qwen"}},
                    ],
                }
            ],
            "local-qwen",
        )

        self.assertEqual(resource.gateway_id, "gateway-local")
        self.assertEqual(resource.resource_id, "local-qwen")

    def test_provider_authentication_failure_does_not_retry_setup_wait(self) -> None:
        class RejectedClient:
            def request(self, path: str) -> tuple[dict[str, object], dict[str, str]]:
                self.path = path
                raise swarm.VifuApiError(401, "unauthorized", "invalid credential")

        with self.assertRaises(swarm.VifuApiError):
            swarm.wait_for_provider(RejectedClient(), "local-qwen", wait_seconds=60)

    def test_route_configuration_uses_one_gateway_backed_chat_capability(self) -> None:
        resource = swarm.ProviderResource("gateway-local", "local-qwen")
        configuration = swarm.route_configuration(
            agent_slug="macbook-swarm-agent-01",
            resource=resource,
            provider_key="local-qwen",
        )

        self.assertEqual(configuration["runtime"]["requestTimeoutMs"], 120_000)
        self.assertEqual(configuration["source"]["resourceId"], "local-qwen")
        self.assertEqual(configuration["capabilities"], [
            {
                "kind": "chat",
                "providerType": "vifu-runtime",
                "providerKey": "local-qwen",
                "resourceId": "local-qwen",
                "config": {"gatewayId": "gateway-local", "source": "macbook-agent-swarm"},
                "inputSchema": {},
                "outputSchema": {},
            }
        ])

    def test_summary_reports_success_and_failure_without_response_contents(self) -> None:
        results = [
            swarm.InvocationResult("agent-a", 1, 100, "ok", "req-a", 4, 6, None),
            swarm.InvocationResult("agent-b", 1, 250, "error", None, None, None, "failed"),
            swarm.InvocationResult("agent-a", 2, 200, "ok", "req-b", 4, 8, None),
        ]

        summary = swarm.benchmark_summary(results, 400)

        self.assertEqual(summary["requests"], 3)
        self.assertEqual(summary["succeeded"], 2)
        self.assertEqual(summary["failed"], 1)
        self.assertEqual(summary["latencyMs"]["p95"], 200)
        self.assertEqual(summary["completionTokens"], 14)
        self.assertEqual(summary["completionTokensPerSecond"], 35.0)

    def test_chart_is_self_contained_and_escapes_request_errors(self) -> None:
        report = {
            "project": "macbook-agent-swarm",
            "providerKey": "local-qwen",
            "agents": 2,
            "requestsPerAgent": 1,
            "maxInFlight": 2,
            "summary": {
                "requests": 2,
                "succeeded": 1,
                "failed": 1,
                "wallTimeMs": 400,
                "latencyMs": {"p95": 250},
                "completionTokensPerSecond": 12.5,
            },
            "results": [
                {"agent": "agent-a", "sequence": 1, "latency_ms": 100, "status": "ok"},
                {
                    "agent": "agent-b",
                    "sequence": 1,
                    "latency_ms": 250,
                    "status": "error",
                    "error": "needs <retry> & attention",
                },
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            report_path = Path(directory) / "report.json"
            chart_path = swarm.default_chart_path(report_path)
            swarm.write_chart(chart_path, report)
            chart = chart_path.read_text(encoding="utf-8")

        self.assertEqual(chart_path.name, "report.html")
        self.assertIn("Latency by request", chart)
        self.assertIn("12.50 tokens/s", chart)
        self.assertIn("needs &lt;retry&gt; &amp; attention", chart)
        self.assertNotIn("https://cdn", chart)


if __name__ == "__main__":
    unittest.main()
