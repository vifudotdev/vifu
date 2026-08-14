from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from vifu.app_store import VifuAppStore


APP_ID = "vifu_app_" + "a" * 64


class VifuAppStoreTests(unittest.TestCase):
    def test_first_open_creates_and_records_a_real_server_app(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = VifuAppStore(directory)
            with mock.patch.object(
                store,
                "_request",
                return_value={
                    "app": {"appId": APP_ID, "slug": "weather-watch", "name": "Weather Watch"}
                },
            ) as request:
                app = store.open("http://127.0.0.1:6790/", "Weather Watch")

            self.assertEqual(app.app_id, APP_ID)
            self.assertEqual(
                request.call_args_list,
                [
                    mock.call(
                        "http://127.0.0.1:6790",
                        "v1/local/apps/open",
                        method="POST",
                        body={"name": "Weather Watch", "appId": None},
                    ),
                ],
            )
            manifest = json.loads(
                (Path(directory) / ".vifu" / "app.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                manifest["servers"]["http://127.0.0.1:6790"]["appId"],
                APP_ID,
            )

    def test_next_open_reuses_the_project_binding(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / ".vifu" / "app.json"
            manifest.parent.mkdir()
            manifest.write_text(
                json.dumps(
                    {
                        "schemaVersion": 1,
                        "name": "Weather Watch",
                        "servers": {
                            "http://127.0.0.1:6790": {
                                "appId": APP_ID,
                                "slug": "weather-watch",
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            store = VifuAppStore(directory)
            with mock.patch.object(
                store,
                "_request",
                return_value={
                    "app": {"appId": APP_ID, "slug": "weather-watch", "name": "Weather Watch"}
                },
            ) as request:
                app = store.open("http://127.0.0.1:6790", "Renamed Locally")

            self.assertEqual(app.app_id, APP_ID)
            request.assert_called_once_with(
                "http://127.0.0.1:6790",
                "v1/local/apps/open",
                method="POST",
                body={"name": "Renamed Locally", "appId": APP_ID},
            )

    def test_each_workspace_has_its_own_app_binding(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            self.assertNotEqual(
                VifuAppStore(first).manifest_path,
                VifuAppStore(second).manifest_path,
            )

    def test_invalid_manifest_is_reported_instead_of_hidden(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / ".vifu" / "app.json"
            manifest.parent.mkdir()
            manifest.write_text("not json", encoding="utf-8")

            with self.assertRaisesRegex(RuntimeError, "manifest is invalid"):
                VifuAppStore(directory).open("http://127.0.0.1:6790", "Broken")


if __name__ == "__main__":
    unittest.main()
