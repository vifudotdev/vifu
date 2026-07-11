# Self-hosting Vifu

The included Docker Compose stack runs one Vifu deployment. It requires no Vifu
account and communicates through ordinary HTTP and WebSocket contracts.

## Services

| Service | Role | Default address |
| --- | --- | --- |
| `postgres` | Durable runtime state | `127.0.0.1:5432` |
| `backend` | `vifu-server` HTTP and WebSocket runtime | `127.0.0.1:6790` |
| `dashboard` | Next.js standalone management console | `127.0.0.1:6791` |

## Start

From the repository root:

```bash
sh scripts/init-self-hosted.sh
docker compose -f self-hosted/docker/docker-compose.yml up --build --wait
```

Open `http://localhost:6791` and verify the runtime:

```bash
curl --fail --silent http://127.0.0.1:6790/health
curl --fail --silent http://127.0.0.1:6790/v1/status
```

PostgreSQL data is stored in the `postgres_data` Docker volume. Restarting or
recreating containers does not delete profiles, bindings, endpoints, keys,
connector sessions, or traces.

## Configuration

The initialization script creates an untracked, mode `0600` `.env` file with
independent random authority values. It refuses to overwrite an existing file.
Compose accepts these values:

| Variable | Purpose |
| --- | --- |
| `VIFU_ADMIN_KEY` | Dashboard and deployment administration |
| `VIFU_CONNECTOR_TOKEN` | Connector WebSocket authentication |
| `VIFU_API_KEY_PEPPER` | One-way endpoint key hashing |
| `DATABASE_URL` | PostgreSQL connection string for `vifu-server` |
| `VIFU_BIND_HOST` | Host interface for published ports |
| `VIFU_SERVER_PORT` | Published runtime port |
| `VIFU_DASHBOARD_PORT` | Published Dashboard port |
| `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD` | Included PostgreSQL container |

The three authority secrets must be independent and contain at least 16
characters. Compose refuses to start when they are missing. `VIFU_ADMIN_KEY`
remains server-side; never give it a `NEXT_PUBLIC_` name.

Vifu depends on PostgreSQL, not a provider-specific API. A PostgreSQL-compatible
managed provider can be used by setting `DATABASE_URL`; the included container
is the default self-host option.

## Connector

Run the connector on the machine that can reach the local OpenClaw gateway:

```bash
VIFU_OPENCLAW_TOKEN=replace-with-your-gateway-token \
sh scripts/dev-connector.sh
```

The connector accepts only loopback OpenClaw URLs. It opens one authenticated
WebSocket to `vifu-server`, discovers agents through OpenClaw's enabled
OpenAI-compatible HTTP surface, and carries concurrent logical endpoint
channels over that connection. Gateways configured without authentication can
omit `VIFU_OPENCLAW_TOKEN`.

Remote `VIFU_SERVER_URL` values must use HTTPS. Plain HTTP is accepted only for
loopback development so connector credentials are never sent over a remote
plaintext WebSocket.

## Exposure

All published ports bind to `127.0.0.1` by default. For remote access, keep
PostgreSQL private and place the Dashboard and runtime behind TLS, request-size
limits, and an external identity layer or trusted private network. Set
`VIFU_BIND_HOST` only after those controls are in place.

The Dashboard's admin key is powerful deployment authority. Rotate it and the
connector token if either appears in logs, shell history, screenshots, or
browser-visible configuration.

## Upgrade And Backup

Build new images and recreate services without deleting the volume:

```bash
docker compose -f self-hosted/docker/docker-compose.yml up -d --build --wait
```

`vifu-server` applies idempotent migrations before accepting traffic. Back up
PostgreSQL before upgrading across releases. Do not use `down --volumes` unless
you intend to delete all deployment data.
