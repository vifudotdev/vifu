from __future__ import annotations

import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace

from vifu import vifu_mobile_ffi as native
from vifu.gateway import VifuGateway, _GatewayCredentials, _gateway_metadata


class _NativeGateway:
    def __init__(self, status: object) -> None:
        self._status = status

    def status(self) -> object:
        return self._status


class GatewayReadinessTests(unittest.TestCase):
    def test_cloud_gateway_identifies_its_code_release(self) -> None:
        with patch.dict("os.environ", {"VIFU_CODE_RELEASE_ID": "rel_alpha"}, clear=False):
            metadata = _gateway_metadata(name="AlphaMind", providers=None)

        self.assertEqual(metadata["codeReleaseId"], "rel_alpha")
        self.assertEqual(metadata["name"], "AlphaMind")

    def test_degraded_gateway_is_ready_for_local_agent_work(self) -> None:
        status = SimpleNamespace(
            state=native.VifuEmbeddedGatewayState.DEGRADED,
            last_error="runtime configuration sync is unavailable",
            authorization=None,
        )
        with tempfile.TemporaryDirectory() as directory:
            gateway = VifuGateway(
                _NativeGateway(status),  # type: ignore[arg-type]
                Path(directory) / "gateway.json",
                _GatewayCredentials(
                    server_url="http://127.0.0.1:6790",
                    machine_private_key="test-private-key",
                ),
            )

            self.assertIs(gateway.wait_until_connected(timeout=0), status)


if __name__ == "__main__":
    unittest.main()
