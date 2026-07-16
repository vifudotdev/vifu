# Scripts

| Script | Purpose |
| --- | --- |
| `init-self-hosted.sh` | Generate independent local secrets in an untracked `.env` |
| `dev-server.sh` | Load `.env.local` and run `vifu-server` |
| `dev-dashboard.sh` | Load `.env.local` and run the single Dashboard |
| `dev-agent-gateway.sh` | Load `.env.local` or `.env` and run the Agent Gateway |
| `local.sh` | Run local PostgreSQL, `vifu-server`, Dashboard, and Agent Gateway together |
| `local-stop.sh` | Stop the local PostgreSQL Compose project without deleting data |
| `mock-openclaw.mjs` | OpenClaw-compatible models and chat fixture |
| `mock-model-provider.mjs` | Deterministic model fixture for real OpenClaw adapter tests |
| `test-self-hosted-e2e.mjs` | Create, verify, and remove E2E runtime resources |
| `run-self-hosted-e2e.sh` | Exercise Agent Gateway concurrency, restart, and persistence; accepts `VIFU_E2E_ENV_FILE` |
| `check-dashboard-boundaries.mjs` | Enforce the one-Dashboard HTTP boundary |
| `check-public-repo.mjs` | Scan public files, tracked output, and repository history |

Scripts must remain local, reproducible, provider-neutral, and free of
credentials or private infrastructure identifiers.
