from __future__ import annotations

import json
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

from vifu import (
    AgentResponse,
    GatewayPairing,
    Vifu,
    VifuRuntime,
    VifuServer,
    VifuServerConfig,
)
from vifu.app import _notify_managed_ready
from vifu.app_store import VifuAppRecord, VifuAppStore
from vifu.gateway import (
    DEFAULT_LOCAL_BOOTSTRAP_TOKEN,
    _local_bootstrap_token,
    _validate_local_server_url,
)


class VifuRuntimeTests(unittest.TestCase):
    def test_app_provider_requires_an_executable_implementation(self) -> None:
        app = Vifu("Provider App")

        with self.assertRaisesRegex(
            ValueError,
            "requires an implementation",
        ):
            app.provider(
                "missing-provider",
                None,
                provider_type="openai-compatible",
                capabilities=("chat",),
            )

    def test_app_declares_private_providers_and_binds_them_to_agent_profiles(self) -> None:
        class FixtureProvider:
            vifu_provider_type = "fixture-chat"
            vifu_capabilities = ("chat",)
            vifu_settings = {"model": "fixture-model"}

            def complete(self, request, *, session_id):
                return {"request": request, "sessionId": session_id}

        app = Vifu("Provider App")
        provider = app.provider(
            "shared-reasoning",
            FixtureProvider(),
            name="Shared Reasoning",
        )
        app.agent(
            "reply",
            lambda _request: {},
            implementation="strands-agents",
            providers={"reasoning": provider},
        )

        self.assertIs(app.providers["shared-reasoning"], provider)
        self.assertEqual(provider.provider_type, "fixture-chat")
        self.assertEqual(provider.capabilities, ("chat",))
        metadata = app._registrations[0][2]["metadata"]
        self.assertEqual(metadata["implementation"], "strands-agents")
        self.assertEqual(
            metadata["providerBindings"],
            {
                "reasoning": {
                    "providerKey": "shared-reasoning",
                    "capability": "chat",
                }
            },
        )

    def test_app_provider_descriptors_do_not_expose_provider_credentials(self) -> None:
        class CredentialProvider:
            vifu_provider_type = "openai-compatible"
            vifu_capabilities = ("chat",)
            vifu_settings = {
                "model": "gpt-test",
                "apiKey": "must-not-escape",
                "headers": {"Authorization": "Bearer must-not-escape-header"},
            }
            vifu_resources = {
                "model": "model:gpt-test",
                "accessToken": "must-not-escape-resource",
            }

            def complete(self, request, *, session_id):
                return {"request": request, "sessionId": session_id}

        app = Vifu("Provider Metadata")
        app.provider("private-model", CredentialProvider())

        descriptors = app._provider_descriptors()

        self.assertEqual(len(descriptors), 1)
        self.assertEqual(descriptors[0]["id"], "private-model")
        self.assertEqual(descriptors[0]["localProviderType"], "openai-compatible")
        self.assertEqual(descriptors[0]["resources"], {"model": "model:gpt-test"})
        self.assertNotIn("apiKey", json.dumps(descriptors))
        self.assertNotIn("Authorization", json.dumps(descriptors))
        self.assertNotIn("token", json.dumps(descriptors).lower())

    def test_app_provider_does_not_become_an_extra_agent_implementation(self) -> None:
        class FixtureProvider:
            vifu_provider_type = "fixture-chat"
            vifu_capabilities = ("chat",)

            def complete(self, request, *, session_id):
                return {"request": request, "sessionId": session_id}

        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Provider Runtime", data_dir=directory, workspace=directory)
            provider = app.provider("shared-reasoning", FixtureProvider())
            app.agent(
                "reply",
                lambda _request: {},
                implementation="strands-agents",
                providers={"reasoning": provider},
            )

            runtime = mock.Mock()
            with (
                mock.patch.object(VifuServer, "ensure", return_value=None),
                mock.patch.object(
                    VifuAppStore,
                    "open",
                    return_value=VifuAppRecord(
                        APP_ID,
                        "provider-runtime",
                        "Provider Runtime",
                    ),
                ),
                mock.patch("vifu.app.VifuRuntime", return_value=runtime),
            ):
                _ = app.runtime

            self.assertEqual(len(runtime.method_calls), 1)
            self.assertEqual(
                runtime.method_calls[0].args[:2],
                ("reply", app._registrations[0][1]),
            )
            app.close()

    def test_high_level_app_registers_and_invokes_a_decorated_agent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Python App Test", data_dir=directory, workspace=directory)

            @app.agent("guide")
            def guide(request):
                return {"text": f"Hello, {request.input['name']}"}

            with _local_app(APP_ID):
                result = app.invoke("guide", {"name": "Ada"})

            self.assertEqual(result.output, {"text": "Hello, Ada"})
            self.assertEqual(app.runtime.app_id, "python-app-test")
            app.close()

    def test_high_level_app_exposes_developer_instructions_to_the_agent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Prompt Test", data_dir=directory, workspace=directory)

            @app.agent("guide", instructions="Answer with one short sentence.")
            def guide(request):
                return {"instructions": request.instructions}

            with _local_app(APP_ID):
                result = app.invoke("guide", {})

            self.assertEqual(
                result.output,
                {"instructions": "Answer with one short sentence."},
            )
            app.close()

    def test_high_level_app_rejects_empty_agent_instructions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Prompt Test", data_dir=directory, workspace=directory)
            app.agent("guide", lambda _request: {}, instructions="  ")

            with _local_app(APP_ID):
                with self.assertRaisesRegex(ValueError, "must not be empty"):
                    _ = app.runtime

            self.assertIsNone(app._runtime)

    def test_high_level_app_requires_a_name(self) -> None:
        with self.assertRaises(ValueError):
            Vifu("  ")

    def test_high_level_app_accepts_a_managed_handler(self) -> None:
        class Handler:
            metadata = {"model": "managed-fixture"}

            def __init__(self):
                self.prepared = 0
                self.closed = 0

            def prepare(self):
                self.prepared += 1

            def __call__(self, request):
                return {"text": request.input["prompt"]}

            def close(self):
                self.closed += 1

        with tempfile.TemporaryDirectory() as directory:
            handler = Handler()
            app = Vifu("Managed Handler", data_dir=directory, workspace=directory)
            app.agent("chat", handler)

            with _local_app(APP_ID):
                result = app.invoke("chat", {"prompt": "hello"})
            app.close()

            self.assertEqual(result.output, {"text": "hello"})
            self.assertEqual(handler.prepared, 1)
            self.assertEqual(handler.closed, 1)

    def test_agent_uses_configuration_declared_by_the_handler(self) -> None:
        class VoiceAgent:
            vifu_name = "Example Voice Agent"
            vifu_endpoint = "voice-transcript"
            vifu_provider_id = "example-voice-service"
            vifu_capability = "speech-to-text"
            vifu_timeout_ms = 45_000
            vifu_metadata = {"framework": "livekit-agents"}
            vifu_instructions = "Emit final transcripts."

            def __call__(self, _request):
                return {}

        app = Vifu("Composable App")
        app.agent("voice", VoiceAgent())

        _, _, options = app._registrations[0]
        self.assertEqual(
            options,
            {
                "name": "Example Voice Agent",
                "endpoint": "voice-transcript",
                "provider_id": "example-voice-service",
                "capability": "speech-to-text",
                "timeout_ms": 45_000,
                "metadata": {"framework": "livekit-agents"},
                "instructions": "Emit final transcripts.",
            },
        )

    def test_agent_binds_and_runs_one_foreground_lifecycle(self) -> None:
        class VoiceAgent:
            def __init__(self):
                self.bound = None
                self.runs = 0
                self.closed = 0

            def __call__(self, _request):
                return {}

            def vifu_bind(self, app, *, agent_id, endpoint):
                self.bound = (app, agent_id, endpoint)

            def vifu_run(self):
                self.runs += 1
                return "voice-finished"

            def close(self):
                self.closed += 1

        voice = VoiceAgent()
        app = Vifu("Composable App")
        app.agent("voice", voice, endpoint="voice-transcript")

        self.assertEqual(voice.bound, (app, "voice", "voice-transcript"))
        with mock.patch.object(app, "connect") as connect:
            result = app.run(connect_timeout=2.0)

        self.assertEqual(result, "voice-finished")
        self.assertEqual(voice.runs, 1)
        self.assertEqual(voice.closed, 1)
        connect.assert_called_once_with(timeout=2.0)

    def test_high_level_app_context_connects_and_closes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Context Lifecycle", data_dir=directory)
            with mock.patch.object(app, "connect") as connect:
                with mock.patch.object(app, "close") as close:
                    with app as active:
                        self.assertIs(active, app)

            connect.assert_called_once_with()
            close.assert_called_once_with()

    def test_high_level_app_cleans_up_after_a_native_connection_error(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Connection Failure", data_dir=directory)
            runtime = mock.Mock()
            runtime.connect_local.side_effect = ValueError("native start failed")
            app._runtime = runtime
            app._app = VifuAppRecord(APP_ID, "connection-failure", "Connection Failure")

            with mock.patch.object(app, "close") as close:
                with self.assertRaisesRegex(ConnectionError, "native start failed"):
                    app.connect()

            close.assert_called_once_with()

    def test_high_level_app_run_serves_without_reading_terminal_input(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Service Lifecycle", data_dir=directory)
            with mock.patch.object(app, "connect") as connect:
                with mock.patch.object(app, "close") as close:
                    with mock.patch("vifu.app.time.sleep", side_effect=KeyboardInterrupt):
                        with mock.patch("builtins.input") as terminal_input:
                            app.run(connect_timeout=2.0)

            connect.assert_called_once_with(timeout=2.0)
            terminal_input.assert_not_called()
            close.assert_called_once_with()

    def test_managed_app_waits_for_one_endpoint_invocation_then_exits(self) -> None:
        with mock.patch.dict(
            "os.environ",
            {"VIFU_GATEWAY_PAIRING_FILE": "/managed/pairing.json"},
            clear=True,
        ):
            app = Vifu("Managed App")
            app._managed_invocation_complete.set()
            with mock.patch.object(app, "connect") as connect:
                with mock.patch.object(app, "close") as close:
                    result = app.run(connect_timeout=4.0)

        self.assertIsNone(result)
        connect.assert_called_once_with(timeout=4.0)
        close.assert_called_once_with()

    def test_managed_app_ignores_the_local_main_callback(self) -> None:
        with mock.patch.dict(
            "os.environ",
            {"VIFU_GATEWAY_PAIRING_FILE": "/managed/pairing.json"},
            clear=True,
        ):
            app = Vifu("Managed App")
            app._managed_invocation_complete.set()
            local_main = mock.Mock()
            with mock.patch.object(app, "connect"):
                with mock.patch.object(app, "close"):
                    result = app.run(local_main)

        self.assertIsNone(result)
        local_main.assert_not_called()

    def test_managed_app_runs_the_same_foreground_agent_lifecycle(self) -> None:
        class VoiceAgent:
            def __init__(self):
                self.runs = 0

            def __call__(self, _request):
                return {}

            def vifu_run(self):
                self.runs += 1
                return "managed-voice-finished"

        with mock.patch.dict(
            "os.environ",
            {
                "VIFU_GATEWAY_PAIRING_FILE": "/managed/pairing.json",
                "VIFU_APP_ID": APP_ID,
            },
            clear=True,
        ):
            voice = VoiceAgent()
            app = Vifu("Managed Voice App")
            app.agent("voice", voice)
            with mock.patch.object(app, "connect") as connect:
                with mock.patch.object(app, "close") as close:
                    result = app.run(connect_timeout=4.0)

        self.assertEqual(result, "managed-voice-finished")
        self.assertEqual(voice.runs, 1)
        connect.assert_called_once_with(timeout=4.0)
        close.assert_called_once_with()

    def test_provider_completion_signals_managed_app_lifecycle(self) -> None:
        completed = threading.Event()
        with tempfile.TemporaryDirectory() as directory:
            runtime = VifuRuntime("managed-lifecycle", data_dir=directory)
            runtime.agent(
                "assistant",
                lambda request: {"text": request.input["text"]},
                _on_complete=completed.set,
            )

            result = runtime.invoke("assistant", {"text": "done"})
            runtime.close()

        self.assertEqual(result.output, {"text": "done"})
        self.assertTrue(completed.is_set())

    def test_managed_ready_notification_uses_scoped_control_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pairing_path = root / "pairing.json"
            pairing_path.write_text(
                json.dumps(
                    {
                        "pairingCode": (
                            "vifu://gateway/enroll?"
                            "server=https%3A%2F%2Fruntime.example&token=vifu_ge_test"
                        )
                    }
                )
            )
            control_path = root / "control.json"
            control_path.write_text(
                json.dumps(
                    {
                        "readyUrl": "https://api.example/v1/vifu/managed/ready",
                        "token": "ready-only-token",
                        "executionId": "execution-123",
                    }
                )
            )
            environment = {
                "VIFU_APP_ID": "managed-app",
                "VIFU_GATEWAY_PAIRING_FILE": str(pairing_path),
                "VIFU_MANAGED_CONTROL_FILE": str(control_path),
            }
            response = mock.MagicMock()
            response.__enter__.return_value.status = 204
            with mock.patch.dict("os.environ", environment, clear=True):
                app = Vifu("Managed App", data_dir=root / "runtime")
                runtime = mock.Mock()
                gateway = mock.Mock()
                runtime.connect.return_value = gateway
                app._runtime = runtime
                with mock.patch("vifu.app.urlopen", return_value=response) as post:
                    app.connect(timeout=4.0)

            request = post.call_args.args[0]
            self.assertEqual(request.full_url, "https://api.example/v1/vifu/managed/ready")
            self.assertEqual(json.loads(request.data), {"executionId": "execution-123"})
            self.assertEqual(request.get_header("Authorization"), "Bearer ready-only-token")
            self.assertEqual(request.get_header("User-agent"), "Vifu-Python-SDK/0.1.8")

    def test_managed_ready_notification_rejects_non_http_loopback_urls(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            control_path = Path(directory) / "control.json"
            control_path.write_text(
                json.dumps(
                    {
                        "readyUrl": "ftp://localhost/v1/vifu/managed/ready",
                        "token": "ready-only-token",
                        "executionId": "execution-123",
                    }
                )
            )
            with mock.patch.dict(
                "os.environ",
                {"VIFU_MANAGED_CONTROL_FILE": str(control_path)},
                clear=True,
            ):
                with self.assertRaisesRegex(ValueError, "HTTPS"):
                    _notify_managed_ready()

    def test_python_provider_invocation_produces_a_trace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = VifuRuntime("python-test", data_dir=directory)
            runtime.agent(
                "guide",
                lambda request: AgentResponse(
                    output={"text": f"Echo: {request.input['prompt']}"},
                    metadata={"model": "python-echo"},
                ),
                metadata={"model": "python-echo"},
            )

            invocation = runtime.invoke("guide", {"prompt": "Hello"})

            self.assertEqual(invocation.output, {"text": "Echo: Hello"})
            traces = runtime.pending_traces()
            self.assertEqual(len(traces), 1)
            self.assertEqual(traces[0]["invocationId"], invocation.invocation_id)
            runtime.close()

    def test_python_provider_reports_typed_stages(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = VifuRuntime("python-stages", data_dir=directory)

            def handler(request):
                with request.trace.stage("decode", metadata={"model": "fixture-model"}):
                    return {"text": "done"}

            runtime.agent("guide", handler)
            result = runtime.invoke("guide", {"prompt": "hello"})

            self.assertEqual(result.output, {"text": "done"})
            self.assertEqual(
                [stage["name"] for stage in result.trace],
                ["decode", "provider.invoke"],
            )
            runtime.close()

    def test_python_provider_exception_fails_without_panicking_worker(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            runtime = VifuRuntime("python-errors", data_dir=directory)

            def broken(_request):
                raise IndexError("missing model choice")

            runtime.agent("broken", broken)
            runtime.agent("healthy", lambda _request: {"text": "still running"})

            with self.assertRaisesRegex(
                RuntimeError,
                "provider broken-provider request failed",
            ):
                runtime.invoke("broken", {})
            result = runtime.invoke("healthy", {})

            self.assertEqual(result.output, {"text": "still running"})
            runtime.close()

    def test_pairing_parser_accepts_direct_and_web_codes(self) -> None:
        direct = GatewayPairing.parse(
            "vifu://gateway/enroll?server=http%3A%2F%2F192.168.1.10%3A6790&token=vifu_ge_test"
        )
        web = GatewayPairing.parse(
            "https://vifu.ai/pair#server=http%3A%2F%2F192.168.1.10%3A6790&token=vifu_ge_test"
        )
        self.assertEqual(direct, web)

    def test_pairing_parser_rejects_an_unrelated_vifu_page(self) -> None:
        with self.assertRaises(ValueError):
            GatewayPairing.parse(
                "https://vifu.ai/docs#server=http%3A%2F%2F127.0.0.1%3A6790&token=vifu_ge_test"
            )

    def test_local_gateway_uses_the_implicit_local_bootstrap_token(self) -> None:
        with mock.patch.dict(
            "os.environ",
            {},
            clear=True,
        ):
            self.assertEqual(_local_bootstrap_token(), DEFAULT_LOCAL_BOOTSTRAP_TOKEN)

    def test_automatic_local_connection_rejects_a_remote_server(self) -> None:
        with self.assertRaises(ValueError):
            _validate_local_server_url("https://api.vifu.dev")

    def test_server_manages_a_native_process(self) -> None:
        fixture = Path(__file__).with_name("server_fixture.py")
        server = VifuServer.start(
            executable=sys.executable,
            arguments=[str(fixture)],
            wait_seconds=0.05,
        )
        self.assertTrue(server.running)
        server.close()
        self.assertFalse(server.running)

    def test_server_ensure_starts_the_bundled_server_in_server_only_mode(self) -> None:
        managed = mock.Mock(running=True)
        with mock.patch.object(VifuServer, "is_ready", side_effect=[False, True]):
            with mock.patch.object(VifuServer, "start", return_value=managed) as start:
                result = VifuServer.ensure("http://127.0.0.1:6799")

        self.assertIs(result, managed)
        arguments = start.call_args.kwargs["arguments"]
        self.assertIn("--server-only", arguments)
        self.assertIn("server.address=http://127.0.0.1:6799", arguments)
        self.assertTrue(start.call_args.kwargs["shared"])

    def test_high_level_app_passes_typed_server_startup_configuration(self) -> None:
        config = VifuServerConfig(
            address="http://127.0.0.1:6799",
            profile="research",
            overrides={"server.deployment_id": "local-research"},
        )
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Configured App", data_dir=directory, server_config=config)

            with mock.patch.object(VifuServer, "ensure", return_value=None) as ensure:
                with mock.patch.object(
                    VifuAppStore,
                    "open",
                    return_value=VifuAppRecord(APP_ID, "configured-app", "Configured App"),
                ):
                    _ = app.runtime

            ensure.assert_called_once_with(
                "http://127.0.0.1:6799",
                profile="research",
                overrides={"server.deployment_id": "local-research"},
            )
            app.close()

    def test_high_level_app_closes_the_server_it_started(self) -> None:
        managed_server = mock.Mock()
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Owned Server", data_dir=directory, workspace=directory)

            with mock.patch.object(VifuServer, "ensure", return_value=managed_server):
                with mock.patch.object(
                    VifuAppStore,
                    "open",
                    return_value=VifuAppRecord(APP_ID, "owned-server", "Owned Server"),
                ):
                    _ = app.runtime

            app.close()
            app.close()

        managed_server.close.assert_called_once_with()

    def test_high_level_app_does_not_close_a_reused_server(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Reused Server", data_dir=directory, workspace=directory)

            with mock.patch.object(VifuServer, "ensure", return_value=None):
                with mock.patch.object(
                    VifuAppStore,
                    "open",
                    return_value=VifuAppRecord(APP_ID, "reused-server", "Reused Server"),
                ):
                    _ = app.runtime

            app.close()

        self.assertIsNone(app._server)

    def test_high_level_app_closes_started_server_when_setup_fails(self) -> None:
        managed_server = mock.Mock()
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Failed Setup", data_dir=directory, workspace=directory)

            with mock.patch.object(VifuServer, "ensure", return_value=managed_server):
                with mock.patch.object(
                    VifuAppStore,
                    "open",
                    side_effect=RuntimeError("app setup failed"),
                ):
                    with self.assertRaisesRegex(RuntimeError, "app setup failed"):
                        _ = app.runtime

        managed_server.close.assert_called_once_with()
        self.assertIsNone(app._server)

    def test_server_config_serializes_overrides_for_the_cli(self) -> None:
        managed = mock.Mock(running=True)
        with mock.patch.object(VifuServer, "is_ready", side_effect=[False, True]):
            with mock.patch.object(VifuServer, "start", return_value=managed) as start:
                VifuServer.ensure(
                    profile="research",
                    overrides={
                        "server.deployment_id": "local-research",
                        "server.dashboard.address": "127.0.0.1:6791",
                    },
                )

        self.assertEqual(
            start.call_args.kwargs["arguments"],
            [
                "--no-browser",
                "--server-only",
                "--profile",
                "research",
                "-c",
                "server.address=http://127.0.0.1:6790",
                "-c",
                'server.deployment_id="local-research"',
                "-c",
                'server.dashboard.address="127.0.0.1:6791"',
            ],
        )


APP_ID = "vifu_app_" + "a" * 64


def _local_app(app_id: str):
    return _LocalAppFixture(app_id)


class _LocalAppFixture:
    def __init__(self, app_id: str):
        self._app_id = app_id
        self._server = mock.patch.object(VifuServer, "ensure", return_value=None)
        self._store = mock.patch.object(
            VifuAppStore,
            "open",
            return_value=VifuAppRecord(self._app_id, "python-app-test", "Python App Test"),
        )

    def __enter__(self):
        self._server.start()
        self._store.start()
        return self

    def __exit__(self, *args):
        self._store.stop()
        self._server.stop()


if __name__ == "__main__":
    unittest.main()
