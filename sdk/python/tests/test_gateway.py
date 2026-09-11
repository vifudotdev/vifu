from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from vifu import vifu_mobile_ffi as native
from vifu.gateway import VifuGateway, _GatewayCredentials


class _NativeGateway:
    def __init__(self, status: object) -> None:
        self._status = status

    def status(self) -> object:
        return self._status


class GatewayReadinessTests(unittest.TestCase):
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
