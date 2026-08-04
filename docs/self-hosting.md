# Self-host Vifu

Self-hosted mode runs Vifu as managed services. The Vifu Server does not expose
the embedded local Dashboard in this mode, and it does not open a browser. The
operations Dashboard is a separate, optional Compose service. `--no-browser`
remains accepted as a compatibility option. Interactive `vifu` opens the local
Dashboard only when you press `B`, and headless `vifu` never opens a browser
automatically.

The Compose backend and Dashboard use separate images. The backend image does
not build or copy the Dashboard bundle; the Full Operations Stack builds the
Dashboard image independently.

## Choose A Deployment Shape

From the repository root, create the deployment environment once:

```bash
cp .env.example .env
```

### Full Operations Stack

Start Vifu with its operations Dashboard:

```bash
docker compose up -d
```

Open `http://localhost:6790`. The same address serves the Dashboard, API, and
Agent Gateway connection.

Read the generated Admin Key and enter it in the Console:

```bash
docker compose exec backend cat /run/vifu/secrets/admin_key
```

The Compose project starts:

- PostgreSQL;
- Vifu Server;
- Vifu Agent Gateway;
- the operations Console.

The Server and Gateway containers use the same `vifu` image with different
runtime configuration files.

### Headless Runtime

Start the Server and Agent Gateway without the Dashboard service:

```bash
docker compose up -d backend agent-gateway
```

Compose starts the required secrets and PostgreSQL dependencies automatically.
The Server API is available on the configured port, normally
`http://localhost:6790`; the `dashboard` service is not started. Operate the
deployment through the Server API using the generated Admin Key.

Read that key when an API client needs deployment-admin access:

```bash
docker compose exec backend cat /run/vifu/secrets/admin_key
```

### Server Only

Start only the Server and its required dependencies when Gateways run on other
machines:

```bash
docker compose up -d backend
```

The backend remains headless. Enroll remote Gateways as described below.

Server and Gateway may also run as roles in the same local `vifu` process. The
combined and Compose configurations use a deployment bootstrap credential
shared only between those managed roles.

## Connect A Remote Gateway

A Gateway running outside the managed deployment enrolls into a project once:

1. Open the project in the Console and select **Deployments**.
2. Create or select a deployment, then choose **Pair gateway**.
3. Set that Server address in the Gateway's `~/.vifu/config.toml`.
4. Provide the displayed one-time token on the Gateway's next start through
   `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE`.

The configuration names both components explicitly. Here the Server is remote
and the Gateway is local, so this process starts only the Gateway:

```toml
[server]
address = "https://api.vifu.ai"

[gateway]
address = "http://localhost:6790"
```

For example, point the file variable at a private temporary file containing
only the token, start `vifu`, then remove the file after enrollment succeeds.
The token is consumed by Server and is not copied into `config.toml`.

Enrollment tokens expire after five minutes, can be used once, and are never
written to the persistent runtime configuration. Issuing a new unused token for
the deployment revokes the previous unused token. A Gateway that has enrolled
reconnects with its stable Machine identity and Server-issued Device Token in
the Server-scoped record inside `~/.vifu/runtime.sqlite`. If authorization is
required again, Vifu prints a Dashboard link and keeps retrying while the
operator reviews the request.

To use a second Vifu CLI only as the remote TUI, configure its remote
`server.address` and omit `[gateway]`. Provide that deployment's admin
credential through `VIFU_ADMIN_KEY` or `VIFU_ADMIN_KEY_FILE`. The CLI opens an
authenticated monitor WebSocket on the same Server origin and receives the
current Gateway snapshot followed by live runtime events.

Each project starts with a `development` deployment. More deployments can use
different Gateways and active Runtime Releases while keeping the same project
contract. The primary deployment serves the existing project endpoint.

When a Server operator enables guest bootstrap, an unpaired Gateway can receive
a temporary project, deployment, project key, and claim token on first
connection. Claiming the project from the Console transfers it to the signed-in
owner without replacing the Gateway identity. Guest projects expire according
to the Server's configured lifetime.

Agent Gateway is a Server transport: it requires a reachable Vifu Server.
Applications that embed `VifuRuntime` register their providers directly as
described in [Embed the runtime](runtime-embedding.md).

## Operate

```bash
docker compose ps
docker compose logs -f backend agent-gateway
docker compose logs -f dashboard # Full Operations Stack only
docker compose up -d
docker compose down
```

`docker compose down` preserves the named PostgreSQL volume. Use normal database
backup procedures before upgrades or destructive maintenance.

## Inspect PostgreSQL

Start the optional, loopback-only pgAdmin service:

```bash
docker compose --profile database-tools up -d pgadmin
```

Open `http://<VIFU_PGADMIN_BIND_HOST>:5050` and sign in with the email
configured by `VIFU_PGADMIN_EMAIL` (default: `pgadmin@vifu.dev`). The bind host
falls back to `VIFU_BIND_HOST`, whose public default is `127.0.0.1`. Read the
generated pgAdmin password from the running container:

```bash
docker compose exec pgadmin cat /run/vifu/secrets/pgadmin_password
```

The `Vifu PostgreSQL` server is registered automatically. Its database password
is loaded from the deployment secret volume and is not stored in the Compose
file. Keep the pgAdmin bind host on loopback or a private operator network such
as Tailscale; do not expose it directly to the public Internet.

## Configure

Copy `.env.example` once and set deployment-local values there. Runtime role and
network settings are defined by `docker-compose.yml`; generated deployment
secrets remain in the named `vifu_secrets` volume across normal restarts.
Set `VIFU_SERVER_API_ADDR` to the HTTPS address used by clients when an ingress,
reverse proxy, or tunnel exposes Vifu outside the local machine.

The corresponding Server configuration uses `server.address` for that one
public origin. In `self-hosted` deployment mode the process owns the Server and
uses its deployment-managed internal port (6790 by default); the internal bind
is not a second client address and does not belong in `config.toml`.

Provider integrations are configured independently. Runtime-owned providers live
in the `providers.json` loaded by the Agent Gateway, while project-local
providers and project assignments live in the Server database. Start with
[providers/README.md](../providers/README.md), then attach the available
provider keys to each project from the Dashboard or API.

## Network Boundary

The default Compose ports bind to loopback. Put a TLS reverse proxy in front of
the Server and Console before exposing them to another machine or the public
Internet. Keep the runtime admin credential on the Console server; browser code
for applications must use project API keys instead.
