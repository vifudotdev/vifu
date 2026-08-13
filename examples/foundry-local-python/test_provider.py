from __future__ import annotations

import tempfile
import unittest
from types import SimpleNamespace

from vifu import VifuRuntime

from provider import register_foundry_agent


class FakeFoundryClient:
    def complete_streaming_chat(self, _messages):
        for content in ("local ", "answer"):
            yield SimpleNamespace(
                choices=[SimpleNamespace(delta=SimpleNamespace(content=content))]
            )


class FoundryLocalAdapterTest(unittest.TestCase):
    def test_streams_a_native_client_through_vifu(self) -> None:
        with tempfile.TemporaryDirectory() as data_dir:
            runtime = VifuRuntime("foundry-test", data_dir=data_dir)
            register_foundry_agent(runtime, FakeFoundryClient(), model="test-model")

            result = runtime.invoke("foundry-chat", {"prompt": "hello"})

            self.assertEqual(result.output, {"text": "local answer"})
            self.assertEqual(
                [stage["name"] for stage in result.trace],
                ["first_token", "decode", "provider.invoke"],
            )
            self.assertEqual(result.metadata["model"], "test-model")


if __name__ == "__main__":
    unittest.main()
