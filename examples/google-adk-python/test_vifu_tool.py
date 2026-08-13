from __future__ import annotations

import tempfile
import unittest
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

from vifu import VifuRuntime


_ADAPTER_PATH = Path(__file__).parent / "vifu_adk_agent" / "vifu_tool.py"
_SPEC = spec_from_file_location("vifu_google_adk_tool", _ADAPTER_PATH)
assert _SPEC is not None and _SPEC.loader is not None
_MODULE = module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
create_vifu_tool = _MODULE.create_vifu_tool


class GoogleAdkToolAdapterTest(unittest.TestCase):
    def test_tool_invokes_the_embedded_runtime(self) -> None:
        with tempfile.TemporaryDirectory() as data_dir:
            runtime = VifuRuntime("google-adk-test", data_dir=data_dir)
            runtime.agent("local", lambda request: {"text": request.input["prompt"]})

            result = create_vifu_tool(runtime, "local")("hello")

            self.assertEqual(result["output"], {"text": "hello"})
            self.assertTrue(result["vifu_invocation_id"])
            self.assertEqual(len(runtime.pending_traces()), 1)


if __name__ == "__main__":
    unittest.main()
