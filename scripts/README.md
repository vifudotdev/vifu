# Scripts

Most users should use the root `package.json` commands:

| Command | Purpose |
| --- | --- |
| `bun run local` | Run local PostgreSQL, `vifu-server`, Dashboard, and Agent Gateway together |
| `bun run local:stop` | Stop the local Compose database without deleting data |
| `bun run self-host` | Start the Docker self-host stack |
| `bun run self-host:stop` | Stop the Docker self-host stack |
| `bun run self-host:logs` | Follow Docker self-host logs |

The files in this directory are implementation details behind those commands
and the release checks:

| Script | Purpose |
| --- | --- |
| `dev-server.sh` | Load `.env.local` and run `vifu-server` |
| `dev-dashboard.sh` | Load `.env.local` and run the Dashboard |
| `dev-agent-gateway.sh` | Load local env and run the Agent Gateway |
| `local.sh` | Implementation for `bun run local` |
| `local-stop.sh` | Implementation for `bun run local:stop` |
| `self-host.sh` | Implementation for `bun run self-host` |
| `mock-openclaw.mjs` | OpenClaw-compatible agent fixture |
| `mock-model-provider.mjs` | Deterministic model fixture for real OpenClaw adapter tests |
| `mock-oidc-provider.mjs` | Local OIDC fixture for auth tests |
| `test-self-hosted-e2e.mjs` | Create, verify, and remove E2E runtime resources |
| `run-self-hosted-e2e.sh` | Exercise Agent Gateway concurrency, restart, and persistence |
| `run-oidc-e2e.sh` | Exercise optional self-host OIDC auth |
| `check-dashboard-boundaries.mjs` | Enforce the one-Dashboard HTTP boundary |
| `check-public-repo.mjs` | Scan public files, tracked output, and repository history |

Scripts must remain local, reproducible, provider-neutral, and free of
credentials or private infrastructure identifiers.
