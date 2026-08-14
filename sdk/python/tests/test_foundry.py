from __future__ import annotations

import tempfile
import unittest
from types import SimpleNamespace

from vifu import VifuRuntime
from vifu.integrations.foundry import trace_foundry_stream


class FakeFoundryClient:
    def __init__(self):
        self.requests = []

    def complete_streaming_chat(self, messages):
        self.requests.append(messages)
        for content in ("local ", "answer"):
            yield SimpleNamespace(
                choices=[SimpleNamespace(delta=SimpleNamespace(content=content))]
            )


class FoundryTracingTest(unittest.TestCase):
    def test_preserves_the_existing_client_call_and_original_chunks(self) -> None:
        with tempfile.TemporaryDirectory() as data_dir:
            runtime = VifuRuntime("foundry-test", data_dir=data_dir)
            client = FakeFoundryClient()

            def researcher(request):
                messages = [{"role": "user", "content": request.input["question"]}]
                native_stream = client.complete_streaming_chat(messages)
                observed_stream = trace_foundry_stream(
                    request,
                    native_stream,
                    model="test-model",
                )
                text = "".join(
                    chunk.choices[0].delta.content or ""
                    for chunk in observed_stream
                )
                return {"answer": text, "source": "foundry-local"}

            runtime.agent("researcher", researcher, capability="research")
            invocation = runtime.invoke("researcher", {"question": "What changed?"})

            self.assertEqual(
                client.requests,
                [[{"role": "user", "content": "What changed?"}]],
            )
            self.assertEqual(
                invocation.output,
                {"answer": "local answer", "source": "foundry-local"},
            )
            self.assertEqual(
                [stage["name"] for stage in invocation.trace],
                ["first_token", "decode", "provider.invoke"],
            )
            runtime.close()


if __name__ == "__main__":
    unittest.main()
