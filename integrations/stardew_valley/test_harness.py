from __future__ import annotations

import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any


sys.path.insert(0, str(Path(__file__).resolve().parent))
import harness  # noqa: E402


class FakeVifuClient:
    def __init__(self) -> None:
        self.project: dict[str, Any] | None = None
        self.provider_attached = False
        self.profiles: list[dict[str, Any]] = [
            {
                "id": "physical-profile-id",
                "slug": "stardew-valley-local-qwen",
                "name": "Stardew Valley Local Qwen",
                "activeVersionId": "physical-version-id",
            },
            {
                "id": "unrelated-profile-id",
                "slug": "main",
                "name": "main",
                "activeVersionId": "unrelated-version-id",
            },
        ]
        self.profile_details: dict[str, dict[str, Any]] = {}
        self.endpoints: list[dict[str, Any]] = [
            {
                "id": "unrelated-endpoint-id",
                "profileId": "unrelated-profile-id",
                "bindingId": "unrelated-binding-id",
                "slug": "main",
            }
        ]
        self.created_projects = 0
        self.created_providers = 0
        self.refreshed_providers = 0
        self.created_endpoints = 0
        self.created_task_profiles = 0
        self.created_task_profile_versions = 0
        self.activated_task_profile_versions = 0
        self.remote_invocation_enabled = False
        self.updated_deployments = 0

    def request(
        self, path: str, *, method: str = "GET", body: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        if path == "/v1/projects" and method == "GET":
            return {"projects": [self.project] if self.project else []}
        if path == "/v1/projects" and method == "POST":
            self.created_projects += 1
            self.project = {
                "id": "project-id",
                "slug": body["slug"],
                "name": body["name"],
                "gatewayId": "unassigned",
            }
            return {"project": self.project}
        if path == "/v1/agent-gateways":
            return {
                "agentGateways": [
                    {
                        "gatewayId": "gateway-local",
                        "status": "connected",
                        "agents": [
                            {
                                "id": "stardew-valley-llama",
                                "status": "connected",
                                "metadata": {"providerKey": "stardew-valley-llama"},
                            },
                            {
                                "id": "main",
                                "status": "connected",
                                "metadata": {"providerKey": "openclaw-local"},
                            },
                        ],
                    }
                ]
            }
        if path == "/v1/projects/project-id" and method == "PATCH":
            assert self.project is not None
            self.project = {**self.project, "gatewayId": body["gatewayId"]}
            return {"project": self.project}
        if path == "/v1/project/stardew-valley/deployments" and method == "GET":
            return {
                "deployments": [
                    {
                        "name": "development",
                        "isPrimary": True,
                        "remoteInvocationEnabled": self.remote_invocation_enabled,
                    }
                ]
            }
        if (
            path == "/v1/project/stardew-valley/deployments/development"
            and method == "PATCH"
        ):
            self.remote_invocation_enabled = body["remoteInvocationEnabled"]
            self.updated_deployments += 1
            return {"deployment": {"name": "development"}}
        if path == "/v1/project/stardew-valley/providers" and method == "GET":
            return {
                "providers": (
                    [
                        {
                            "providerKey": "stardew-valley-llama",
                            "sourceKind": "custom",
                            "sourceKey": "stardew-valley-llama",
                        }
                    ]
                    if self.provider_attached
                    else []
                )
            }
        if path == "/v1/project/stardew-valley/providers" and method == "POST":
            self.provider_attached = True
            self.created_providers += 1
            return {"addedAgents": 1}
        if (
            path == "/v1/project/stardew-valley/providers/stardew-valley-llama/test"
            and method == "POST"
        ):
            self.refreshed_providers += 1
            return {"addedAgents": 0}
        if path == "/v1/project/stardew-valley/profiles" and method == "GET":
            return {"profiles": self.profiles}
        if path == "/v1/project/stardew-valley/profiles" and method == "POST":
            profile_id = f"task-profile-{self.created_task_profiles}"
            version_id = f"task-version-{self.created_task_profiles}"
            profile = {
                "id": profile_id,
                "slug": body["slug"],
                "name": body["name"],
                "activeVersionId": version_id,
            }
            self.profiles.append(profile)
            self.profile_details[profile_id] = self._profile_detail(
                profile, version_id, body
            )
            self.created_task_profiles += 1
            return {"profile": profile}
        if path.startswith("/v1/project/stardew-valley/profiles/"):
            parts = path.split("/")
            profile_id = parts[5]
            if len(parts) == 6 and method == "GET":
                return self.profile_details[profile_id]
            if len(parts) == 7 and parts[6] == "versions" and method == "POST":
                self.created_task_profile_versions += 1
                version_id = f"updated-version-{self.created_task_profile_versions}"
                profile = next(item for item in self.profiles if item["id"] == profile_id)
                self.profile_details[profile_id] = self._profile_detail(
                    profile, version_id, body
                )
                return {"version": {"id": version_id}}
            if (
                len(parts) == 9
                and parts[6] == "versions"
                and parts[8] == "activate"
                and method == "POST"
            ):
                self.activated_task_profile_versions += 1
                profile = next(item for item in self.profiles if item["id"] == profile_id)
                profile["activeVersionId"] = parts[7]
                return {"profile": profile}
        if path == "/v1/project/stardew-valley/bindings":
            return {
                "bindings": [
                    {
                        "id": "binding-id",
                        "profileId": "physical-profile-id",
                        "gatewayId": "gateway-local",
                        "agentId": "stardew-valley-llama",
                        "config": {"providerKey": "stardew-valley-llama"},
                    },
                    {
                        "id": "unrelated-binding-id",
                        "profileId": "unrelated-profile-id",
                        "gatewayId": "gateway-local",
                        "agentId": "main",
                        "config": {"providerKey": "openclaw-local"},
                    },
                ]
            }
        if path == "/v1/project/stardew-valley/endpoints" and method == "GET":
            return {"endpoints": self.endpoints}
        if path == "/v1/project/stardew-valley/endpoints" and method == "POST":
            endpoint = {
                "id": "endpoint-id",
                "profileId": body["profileId"],
                "bindingId": body["bindingId"],
                "slug": body["slug"],
            }
            self.endpoints.append(endpoint)
            self.created_endpoints += 1
            return {"endpoint": endpoint}
        if path == "/stardew-valley/v1/models":
            return {
                "data": [{"id": profile["slug"]} for profile in self.profiles]
            }
        raise AssertionError(f"unexpected request: {method} {path}")

    @staticmethod
    def _profile_detail(
        profile: dict[str, Any], version_id: str, body: dict[str, Any]
    ) -> dict[str, Any]:
        return {
            "profile": {**profile, "activeVersionId": version_id},
            "versions": [
                {
                    "version": {
                        "id": version_id,
                        "persona": body["persona"],
                        "runtime": body["runtime"],
                        "presentation": body["presentation"],
                        "source": body["source"],
                    },
                    "capabilities": body["capabilities"],
                }
            ],
            "rollout": [],
        }


class BootstrapTests(unittest.TestCase):
    def test_project_configuration_is_idempotent(self) -> None:
        client = FakeVifuClient()
        task_models = [
            "stardew-valley-farming-0",
            "stardew-valley-farming-1",
        ]

        first = harness.ensure_project_configuration(
            client,
            project_slug="stardew-valley",
            project_name="stardew_valley",
            chat_provider_key="stardew-valley-llama",
            embedding_provider_key="stardew-valley-llama",
            requested_gateway_id=None,
            task_models=task_models,
        )
        second = harness.ensure_project_configuration(
            client,
            project_slug="stardew-valley",
            project_name="stardew_valley",
            chat_provider_key="stardew-valley-llama",
            embedding_provider_key="stardew-valley-llama",
            requested_gateway_id=None,
            task_models=task_models,
        )

        self.assertEqual(first.created_endpoints, 1)
        self.assertEqual(second.created_endpoints, 0)
        self.assertEqual(first.created_task_profiles, 2)
        self.assertEqual(second.created_task_profiles, 0)
        self.assertEqual(second.updated_task_profiles, 0)
        self.assertEqual(second.physical_agent_count, 1)
        self.assertEqual(second.task_model_count, 2)
        self.assertEqual(second.models, tuple(task_models))
        self.assertEqual(second.physical_endpoint_count, 1)
        self.assertEqual(client.created_projects, 1)
        self.assertEqual(client.created_providers, 1)
        self.assertEqual(client.refreshed_providers, 1)
        self.assertEqual(client.created_endpoints, 1)
        self.assertEqual(client.created_task_profiles, 2)
        self.assertEqual(client.created_task_profile_versions, 0)
        self.assertEqual(client.activated_task_profile_versions, 0)
        self.assertEqual(client.updated_deployments, 1)

    def test_project_configuration_refreshes_a_stale_task_route(self) -> None:
        client = FakeVifuClient()
        task_models = ["stardew-valley-farming-0"]
        harness.ensure_project_configuration(
            client,
            project_slug="stardew-valley",
            project_name="stardew_valley",
            chat_provider_key="stardew-valley-llama",
            embedding_provider_key="stardew-valley-llama",
            requested_gateway_id=None,
            task_models=task_models,
        )
        task_profile = next(
            profile for profile in client.profiles if profile["slug"] == task_models[0]
        )
        client.profile_details[task_profile["id"]]["versions"][0]["version"][
            "source"
        ]["gatewayId"] = "stale-gateway"

        refreshed = harness.ensure_project_configuration(
            client,
            project_slug="stardew-valley",
            project_name="stardew_valley",
            chat_provider_key="stardew-valley-llama",
            embedding_provider_key="stardew-valley-llama",
            requested_gateway_id=None,
            task_models=task_models,
        )
        stable = harness.ensure_project_configuration(
            client,
            project_slug="stardew-valley",
            project_name="stardew_valley",
            chat_provider_key="stardew-valley-llama",
            embedding_provider_key="stardew-valley-llama",
            requested_gateway_id=None,
            task_models=task_models,
        )

        self.assertEqual(refreshed.updated_task_profiles, 1)
        self.assertEqual(stable.updated_task_profiles, 0)
        self.assertEqual(client.created_task_profile_versions, 1)
        self.assertEqual(client.activated_task_profile_versions, 1)

    def test_private_env_is_replaced_with_private_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".env.local"
            harness.write_private_env(
                path,
                {
                    "OPENAI_BASE_URL": "http://127.0.0.1:6790/demo/v1",
                    "OA_OPENAI_KEY": "synthetic-project-key",
                },
            )

            self.assertEqual(
                harness.read_env_file(path)["OA_OPENAI_KEY"],
                "synthetic-project-key",
            )
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_private_env_round_trips_shell_values_with_spaces(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".env.local"
            app_path = "/Applications/Steam Library/Stardew Valley/StardewModdingAPI"
            harness.write_private_env(path, {"STARDEW_APP_PATH": app_path})

            result = subprocess.run(
                [
                    "/bin/sh",
                    "-c",
                    '. "$1"; printf "%s" "$STARDEW_APP_PATH"',
                    "sh",
                    str(path),
                ],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.stdout, app_path)
            self.assertEqual(harness.read_env_file(path)["STARDEW_APP_PATH"], app_path)

    def test_stardojo_env_values_use_original_star_dojo_names(self) -> None:
        values = harness.stardojo_env_values(
            existing={"STARDEW_APP_PATH": "/tmp/StardewModdingAPI"},
            project_api_key="synthetic-project-key",
            project_base_url="http://127.0.0.1:6790/stardew-valley/v1",
        )

        self.assertEqual(values["OA_OPENAI_KEY"], "synthetic-project-key")
        self.assertEqual(
            values["OPENAI_BASE_URL"],
            "http://127.0.0.1:6790/stardew-valley/v1",
        )
        self.assertEqual(values["STARDEW_APP_PATH"], "/tmp/StardewModdingAPI")
        self.assertNotIn("VIFU_OPENAI_BASE_URL", values)
        self.assertNotIn("VIFU_PROJECT_API_KEY", values)
        self.assertNotIn("VIFU_MODEL", values)

    def test_multiple_matching_gateways_require_an_explicit_choice(self) -> None:
        gateway = {
            "status": "connected",
            "agents": [
                {
                    "id": "agent",
                    "status": "connected",
                    "metadata": {"providerKey": "provider"},
                }
            ],
        }
        gateways = [
            {**gateway, "gatewayId": "gateway-a"},
            {**gateway, "gatewayId": "gateway-b"},
        ]

        with self.assertRaisesRegex(RuntimeError, "use --gateway-id"):
            harness.select_provider_gateway(gateways, "provider", None, None)

        selected, _ = harness.select_provider_gateway(
            gateways, "provider", "gateway-b", None
        )
        self.assertEqual(selected["gatewayId"], "gateway-b")


if __name__ == "__main__":
    unittest.main()
