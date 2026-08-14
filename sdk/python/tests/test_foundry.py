from __future__ import annotations

import tempfile
import unittest
from types import SimpleNamespace

from vifu import VifuRuntime
from vifu.integrations.foundry import FoundryLocal


class FakeFoundryClient:
    def __init__(self):
        self.requests = []

    def complete_streaming_chat(self, messages):
        self.requests.append(messages)
        for content in ("local ", "answer"):
            yield SimpleNamespace(
                choices=[SimpleNamespace(delta=SimpleNamespace(content=content))]
            )


class FoundryLocalTest(unittest.TestCase):
    def test_manages_the_foundry_model_lifecycle(self) -> None:
        client = FakeFoundryClient()

        class Model:
            def __init__(self):
                self.downloaded = 0
                self.loaded = 0
                self.unloaded = 0

            def download(self, _progress):
                self.downloaded += 1

            def load(self):
                self.loaded += 1

            def unload(self):
                self.unloaded += 1

            def get_chat_client(self):
                return client

        model = Model()
        catalog = SimpleNamespace(get_model=lambda _alias: model)
        manager = SimpleNamespace(
            catalog=catalog,
            download_and_register_eps=lambda **_kwargs: None,
        )
        integration = FoundryLocal("test-model", manager=manager)

        integration.prepare()
        integration.prepare()
        integration.close()

        self.assertEqual(model.downloaded, 1)
        self.assertEqual(model.loaded, 1)
        self.assertEqual(model.unloaded, 1)

    def test_streams_chat_and_preserves_session_history(self) -> None:
        with tempfile.TemporaryDirectory() as data_dir:
            runtime = VifuRuntime("foundry-test", data_dir=data_dir)
            client = FakeFoundryClient()
            runtime.agent("chat", FoundryLocal("test-model", client=client))

            first = runtime.invoke("chat", {"prompt": "hello"}, session_id="conversation")
            second = runtime.invoke(
                "chat",
                {"prompt": "continue"},
                session_id="conversation",
            )

            self.assertEqual(first.output, {"text": "local answer"})
            self.assertEqual(
                [stage["name"] for stage in first.trace],
                ["first_token", "decode", "provider.invoke"],
            )
            self.assertEqual(first.metadata["model"], "test-model")
            self.assertEqual(
                client.requests[1],
                [
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "local answer"},
                    {"role": "user", "content": "continue"},
                ],
            )
            self.assertEqual(len(second.state["messages"]), 4)


if __name__ == "__main__":
    unittest.main()
