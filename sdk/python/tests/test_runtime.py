from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from vifu import AgentResponse, GatewayPairing, Vifu, VifuRuntime, VifuServer
from vifu.app_store import VifuAppRecord, VifuAppStore
from vifu.gateway import (
    DEFAULT_LOCAL_BOOTSTRAP_TOKEN,
    _local_bootstrap_token,
    _validate_local_server_url,
)


class VifuRuntimeTests(unittest.TestCase):
    def test_high_level_app_registers_and_invokes_a_decorated_agent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Python App Test", data_dir=directory, workspace=directory)

            @app.agent("guide")
            def guide(request):
                return {"text": f"Hello, {request.input['name']}"}

            with _local_app(APP_ID):
                result = app.invoke("guide", {"name": "Ada"})

            self.assertEqual(result.output, {"text": "Hello, Ada"})
            self.assertEqual(app.runtime.app_id, APP_ID)
            app.close()

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

    def test_high_level_app_context_connects_and_closes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Context Lifecycle", data_dir=directory)
            with mock.patch.object(app, "connect") as connect:
                with mock.patch.object(app, "close") as close:
                    with app as active:
                        self.assertIs(active, app)

            connect.assert_called_once_with()
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
