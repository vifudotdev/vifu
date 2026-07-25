# Self-host Vifu

## Start

From the repository root:

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`.

The Compose project starts:

- PostgreSQL;
- Vifu Server;
- Vifu Agent Gateway;
- the operations Console.

The Server and Gateway containers use the same `vifu` image with different
runtime configuration files.

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
must use project API keys instead.
