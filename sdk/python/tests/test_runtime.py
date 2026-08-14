from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from vifu import AgentResponse, GatewayPairing, Vifu, VifuRuntime, VifuServer
from vifu.gateway import (
    DEFAULT_LOCAL_BOOTSTRAP_TOKEN,
    _local_bootstrap_token,
    _validate_local_server_url,
)


class VifuRuntimeTests(unittest.TestCase):
    def test_high_level_app_registers_and_invokes_a_decorated_agent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            app = Vifu("Python App Test", data_dir=directory)

            @app.agent("guide")
            def guide(request):
                return {"text": f"Hello, {request.input['name']}"}

            result = app.invoke("guide", {"name": "Ada"})

            self.assertEqual(result.output, {"text": "Hello, Ada"})
            self.assertTrue(app.runtime.app_id.startswith("python-python-app-test-"))

    def test_high_level_app_requires_a_name(self) -> None:
        with self.assertRaises(ValueError):
            Vifu("  ")

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


if __name__ == "__main__":
    unittest.main()
