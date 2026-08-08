# Topology Protocol Live Testing

Vifu has a protocol-level live-test matrix for the boundaries shared by CLI,
embedded, mobile, Godot, self-hosted, and hosted deployments. Run it from the
repository root:

```bash
scripts/test-topologies.sh
```

The command starts real loopback HTTP and WebSocket listeners on random ports.
It exercises Gateway authorization, session reconnect, Server monitoring,
project monitor isolation, multi-device deployment association, logical-profile
to physical-resource filtering, Agent invocation transport, Guest bootstrap,
one-time enrollment, concurrent enrollment, Runtime Distribution installation
and device limits, telemetry retry, embedded content-consent defaults,
enrollment-specific TUI dismissal, CLI topology selection, and private LAN TLS
configuration.

The tests use a separate temporary home, temporary directory, and SQLite
database for each case. Vifu environment variables and the user's
`~/.vifu/config.toml` are not inherited. Cargo build output is shared because
it is not Runtime state. A failed case does not prevent the remaining cases
from running.

Every run creates a new directory under `target/topology-live/` containing:

- `report.json` for automation;
- `junit.xml` for CI test reporting;
- one complete log per case under `logs/`.

Set `VIFU_TOPOLOGY_REPORT_DIR` to place the run directories elsewhere. CI uses
the same script and uploads the whole report directory even when a case fails.

## What This Matrix Proves

The matrix tests the common lower-level path:

```text
Runtime-compatible Agent
  -> Gateway WebSocket
  -> Vifu Server state
  -> authenticated Monitor WebSocket
```

It deliberately does not assert Android, iOS, Godot, or TUI presentation.
Those hosts can change their UI while keeping this protocol contract. Platform
tests remain responsible for camera QR behavior, verified App Links, native
credential storage, and visual rendering.

## Docker Release Verification

The topology matrix does not require Docker and runs on GitHub-hosted Linux.
The heavier self-hosted release test remains available separately:

```bash
scripts/run-self-hosted-e2e.sh
```

That workflow builds the complete Compose stack, exercises the Dashboard and
PostgreSQL, restarts services, verifies durable state and Gateway session
resume, and removes only the resources it created. To run it through a remote
Docker engine, select an existing Docker Context without changing the script:

```bash
DOCKER_CONTEXT="$YOUR_DOCKER_CONTEXT" scripts/run-self-hosted-e2e.sh
```

The lightweight topology matrix is the ordinary CI gate. The Docker workflow
is the complete self-hosted release gate.
