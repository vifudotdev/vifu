# Self-host Vifu

## Start

From the repository root:

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`.

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

Server and Gateway may also run as roles in the same local `vifu` process. The
combined and Compose configurations use a deployment bootstrap credential
shared only between those managed roles.

## Connect A Remote Gateway

A Gateway running outside the managed deployment enrolls into a project once:

1. Open the project in the Console and select **Deployments**.
2. Create or select a deployment, then choose **Pair gateway**.
3. Set that Server URL in the Gateway's `~/.vifu/config.json`.
4. Provide the displayed one-time token on the Gateway's next start through
   `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE`.

For example, point the file variable at a private temporary file containing
only the token, start `vifu`, then remove the file after enrollment succeeds.
The token is consumed by Server and is not copied into `config.json`.

Enrollment tokens expire after five minutes, can be used once, and are never
written to the persistent runtime configuration. Issuing a new unused token for
the deployment revokes the previous unused token. A Gateway that has enrolled
reconnects with its stable Machine identity and Server-issued Device Token in
the Server-scoped record inside `~/.vifu/runtime.sqlite`. If authorization is
required again, Vifu prints a Dashboard link and keeps retrying while the
operator reviews the request.

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
docker compose logs -f backend agent-gateway dashboard
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

Provider integrations are configured independently in the runtime Provider
registry. Start with [providers/README.md](../providers/README.md), then manage
the registry from the Console when it is mounted with the runtime process, or
edit the `providers.json` used by the Agent Gateway directly.

## Network Boundary

The default Compose ports bind to loopback. Put a TLS reverse proxy in front of
the Server and Console before exposing them to another machine or the public
Internet. Keep the runtime admin credential on the Console server; browser code
for applications must use project API keys instead.
