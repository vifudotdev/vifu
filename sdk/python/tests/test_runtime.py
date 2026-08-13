from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
import sys

from vifu import AgentResponse, GatewayPairing, VifuRuntime, VifuServer


class VifuRuntimeTests(unittest.TestCase):
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
