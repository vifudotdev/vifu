#!/usr/bin/env python3
"""Exercise two stable Python Apps against an isolated Vifu Server."""

from __future__ import annotations

import json
import os
import tempfile
import time
from pathlib import Path
from urllib.request import urlopen

from vifu import Vifu


def read_json(url: str) -> dict:
    with urlopen(url, timeout=2.0) as response:
        return json.loads(response.read())


def wait_for(url: str, key: str, count: int, timeout: float = 10.0) -> list[dict]:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        values = read_json(url).get(key, [])
        if len(values) >= count:
            return values
        time.sleep(0.1)
    raise AssertionError(f"{url} did not report {count} {key}")


def register_workers(app: Vifu, prefix: str) -> None:
    for role in ("researcher", "publisher"):
        agent_id = f"{prefix}-{role}"

        def worker(request, current_role=role):
            with request.trace.stage("validate", metadata={"role": current_role}):
                return {"role": current_role, "input": request.input}

        app.agent(agent_id, worker, capability="workflow")


def main() -> None:
    server_url = os.environ["VIFU_E2E_SERVER_URL"].rstrip("/")
    with tempfile.TemporaryDirectory(prefix="vifu-python-apps-") as directory:
        root = Path(directory)
        alpha = Vifu(
            "E2E Alpha",
            workspace=root / "alpha",
            data_dir=root / "alpha-data",
            server_url=server_url,
        )
        beta = Vifu(
            "E2E Beta",
            workspace=root / "beta",
            data_dir=root / "beta-data",
            server_url=server_url,
        )
        (root / "alpha").mkdir()
        (root / "beta").mkdir()
        register_workers(alpha, "alpha")
        register_workers(beta, "beta")
        alpha.connect()
        beta.connect()

        alpha_id = alpha.app_id
        beta_id = beta.app_id
        assert alpha_id != beta_id
        assert alpha.invoke("alpha-researcher", {"company": "Arm"}).app_id == alpha_id
        assert alpha.invoke("alpha-publisher", {"channel": "brief"}).app_id == alpha_id
        assert beta.invoke("beta-researcher", {"topic": "devices"}).app_id == beta_id
        assert beta.invoke("beta-publisher", {"channel": "game"}).app_id == beta_id

        dashboard_api = f"{server_url}/api/runtime"
        apps = read_json(f"{dashboard_api}/apps")["apps"]
        by_id = {app["appId"]: app for app in apps}
        assert alpha_id in by_id and beta_id in by_id
        alpha_slug = by_id[alpha_id]["slug"]
        beta_slug = by_id[beta_id]["slug"]
        wait_for(f"{dashboard_api}/apps/{alpha_slug}/agents", "agents", 2)
        wait_for(f"{dashboard_api}/apps/{beta_slug}/agents", "agents", 2)
        wait_for(f"{dashboard_api}/apps/{alpha_slug}/traces", "traces", 2)
        wait_for(f"{dashboard_api}/apps/{beta_slug}/traces", "traces", 2)

        alpha.close()
        beta.close()
        del alpha

        restarted = Vifu(
            "E2E Alpha",
            workspace=root / "alpha",
            data_dir=root / "alpha-data",
            server_url=server_url,
        )
        register_workers(restarted, "alpha")
        restarted.connect()
        assert restarted.app_id == alpha_id
        assert restarted.invoke("alpha-researcher", {"restart": True}).app_id == alpha_id
        restarted.close()

        print(json.dumps({
            "apps": 2,
            "agentsPerApp": 2,
            "stableRestart": True,
        }))


if __name__ == "__main__":
    main()
