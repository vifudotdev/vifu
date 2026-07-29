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

1. The project owner exchanges their access token for a short-lived Vifu
   deployment credential through `POST /v1/auth/exchange`.
2. The owner uses that credential to call
   `POST /v1/project/{slug}/agent-gateway-enrollments`.
3. The Gateway receives the returned one-time token through
   `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN` or
   `VIFU_AGENT_GATEWAY_ENROLLMENT_TOKEN_FILE` on its first start.
4. Gateway consumes the token, registers its own long-lived credential, and
   stores that credential in a session file scoped to the Server URL.

Enrollment tokens expire after five minutes, can be used once, and are never
written to the persistent runtime configuration. Issuing a new unused token for
the project revokes the previous unused token. A Gateway that has enrolled can
connect again using its stored credential.

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

## Configure

Copy `.env.example` once and set deployment-local values there. Runtime role and
network settings are defined by `docker-compose.yml`; generated deployment
secrets remain in the named `vifu_secrets` volume across normal restarts.

Provider integrations are configured independently. Start with
[providers/README.md](../providers/README.md), then add a provider from the
Console or `~/.vifu/providers.json`.

## Network Boundary

The default Compose ports bind to loopback. Put a TLS reverse proxy in front of
the Server and Console before exposing them to another machine or the public
Internet. Keep the runtime admin credential on the Console server; browser code
for applications must use project API keys instead.
