# Self-host Vifu

Self-hosted mode runs Vifu with Docker Compose. You can start the full stack, a
headless Runtime, or only the Server.

The operations Dashboard is a separate Compose service. The Vifu Server does
not include the embedded local Dashboard in self-hosted mode.
`--no-browser` remains available for compatibility.

The Compose backend and Dashboard use separate images. The backend image does
not contain the Dashboard bundle. The Full Operations Stack builds the
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

The full stack starts:

- PostgreSQL
- Vifu Server
- Vifu Agent Gateway
- the operations Console

The Server and Gateway containers use the same `vifu` image with different
runtime configuration files.

### Headless Runtime

Start the Server and Agent Gateway without the Dashboard service:

```bash
docker compose up -d backend agent-gateway
```

Compose starts the required secrets and PostgreSQL dependencies automatically.
The Server API is available on the configured port. The default address is
`http://localhost:6790`. The `dashboard` service does not start. Operate the
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

Server and Gateway can also run as roles in the same local `vifu` process. The
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

Put the token in a private temporary file. Set the file variable to that path.
Then start `vifu`. Remove the file after enrollment succeeds.
The token is consumed by Server and is not copied into `config.toml`.

An enrollment token expires after five minutes and works once. Vifu does not
write it to persistent Runtime configuration. A new token revokes the previous
unused token.

After enrollment, the Gateway reconnects with its stable Machine identity and
Server-issued Device Token. Vifu stores this Server-specific record in
`~/.vifu/runtime.sqlite`.

If the Gateway needs authorization again, Vifu prints a Dashboard link. It
keeps retrying while the operator reviews the request.

To use a second Vifu CLI as the remote TUI, configure its remote
`server.address`. Omit `[gateway]` for monitor-only operation. Keep a local
Gateway if this computer also hosts Agents.

Provide a project API key through `VIFU_MONITOR_KEY` or
`VIFU_MONITOR_KEY_FILE`. The key must have project read access. The Server
filters the monitor stream to that project.

Deployment operators can use `VIFU_ADMIN_KEY` or `VIFU_ADMIN_KEY_FILE` for
deployment-wide monitoring.

The TUI and Gateway open separate Server connections. Gateway enrollment does
not authenticate the TUI. See
[Runtime topology, monitoring, and Gateway enrollment](topology-and-pairing.md)
for the four placement combinations and complete credential table.

Each project starts with a `development` deployment. More deployments can use
different Gateways and active Runtime Releases while keeping the same project
contract. The primary deployment serves the existing project endpoint.

Guest bootstrap is optional. If an operator enables it, an unpaired Gateway can
receive a temporary project and deployment. It also receives a project key and
claim token.

The Console can transfer the Guest project to a signed-in owner. This transfer
does not replace the Gateway identity. Guest projects use the configured
lifetime.

Project enrollment does not create a Guest project. The managed deployment
bootstrap credential also does not create one.

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
comes from the deployment secret volume. The Compose file does not store this
password.

Keep the pgAdmin bind host on loopback or a private operator network. Do not
expose pgAdmin directly to the public Internet.

## Configure

Copy `.env.example` once and set deployment-local values there.
`docker-compose.yml` defines the Runtime roles and network configuration.
Generated secrets remain in the `vifu_secrets` volume across normal restarts.
Set `VIFU_SERVER_API_ADDR` to the public HTTPS address. Clients use this address
through an ingress, reverse proxy, or tunnel.

The corresponding Server configuration uses `server.address` for that one
public origin. In `self-hosted` deployment mode the process owns the Server and
uses its deployment-managed internal port (6790 by default). The internal bind
is not a second client address and does not belong in `config.toml`.

Provider integrations are configured independently. Runtime-owned providers live
in the `providers.json` loaded by the Agent Gateway, while project-local
providers and project assignments live in the Server database. Start with
[providers/README.md](../providers/README.md), then attach the available
provider keys to each project from the Dashboard or API.

## Network Boundary

The default Compose ports bind to loopback. Add a TLS reverse proxy before you
expose the Server and Console. Keep the Runtime Admin Key on the Console server.
Application browser code must use project API keys instead.
