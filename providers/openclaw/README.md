# OpenClaw Provider

Use this guide to connect an OpenClaw Gateway to Vifu and make its agents
available in the Vifu Dashboard.

Vifu does not run OpenClaw. OpenClaw keeps its own Gateway, model credentials,
workspaces, tools, memory, and agent definitions. Vifu connects to the OpenClaw
Gateway, discovers the available agents, and lets you expose them through Vifu
projects, endpoints, keys, and logs.

## Fast Path

OpenClaw's primary install is the local daemon flow. For a Docker-based Vifu
self-host test, use OpenClaw's Docker setup script from an OpenClaw checkout:

```bash
cd <openclaw>
./scripts/docker/setup.sh
docker compose ps
```

Then register that OpenClaw Gateway in Vifu:

```bash
mkdir -p ~/.vifu
cp <vifu>/providers/openclaw/providers.example.json ~/.vifu/providers.json
```

OpenClaw protects its Gateway with its own Gateway token. In OpenClaw's Docker
setup, that token is generated and stored by OpenClaw. Put the same token in
Vifu's provider registry:

```text
~/.vifu/providers.json
```

Edit `auth.token` and keep the file local to your self-host deployment.

Start or restart Vifu:

```bash
cd <vifu>
docker compose up -d
docker compose logs --tail=60 agent-gateway
```

Expected log output:

```text
OpenClaw provider openclaw-local: online at host.docker.internal:18789
Agents: N discovered
Agent Gateway: connected
```

Open the Dashboard:

```text
http://localhost:6791/project
```

## Integration Boundary

OpenClaw is a provider. Vifu is the endpoint runtime.

Provider registration is kept outside the core Vifu self-host configuration.
The Vifu self-host stack uses the same server, dashboard, database, and Agent
Gateway services regardless of which provider registrations are present.

The only Vifu-side file is:

```text
~/.vifu/providers.json
```

The default example points from the Vifu Docker stack to an OpenClaw Gateway
running on the host:

```json
{
  "providers": [
    {
      "key": "openclaw-local",
      "type": "openclaw",
      "url": "http://host.docker.internal:18789",
      "auth": {
        "token": "replace-with-openclaw-gateway-token"
      }
    }
  ]
}
```

Use `host.docker.internal` here because the Vifu Agent Gateway runs inside a
Docker container. From inside that container, `127.0.0.1` means the Vifu
container itself, not the host machine.

## Existing OpenClaw Gateway

For an existing OpenClaw deployment, register its Gateway in Vifu:

```bash
mkdir -p ~/.vifu
cp <vifu>/providers/openclaw/providers.example.json ~/.vifu/providers.json
```

Then put the OpenClaw Gateway token into:

```json
"auth": {
  "token": "replace-with-openclaw-gateway-token"
}
```

If your OpenClaw Gateway uses a different host or port, edit the `url` in
`~/.vifu/providers.json`.

If your OpenClaw Gateway has no token, remove the `auth` object from
`~/.vifu/providers.json`.

Restart Vifu:

```bash
docker compose up -d
```

## Everyday Restart

For normal local testing, keep the same containers and volumes. Do not delete
them.

```bash
cd <openclaw>
docker compose up -d

cd <vifu>
docker compose up -d
```

Check the Vifu side:

```bash
cd <vifu>
docker compose logs --tail=60 agent-gateway
```

Check the OpenClaw side:

```bash
cd <openclaw>
docker compose ps
curl -sS http://127.0.0.1:18789/healthz
```

## Stop OpenClaw

Stop only OpenClaw:

```bash
cd <openclaw>
docker compose stop
```

Vifu continues running after OpenClaw is stopped. To disable this provider
registration, remove:

```text
~/.vifu/providers.json
```

Then restart Vifu:

```bash
cd <vifu>
docker compose up -d
```

## Troubleshooting

No agents in Vifu:

```bash
cd <vifu>
docker compose logs --tail=80 agent-gateway
```

OpenClaw not reachable:

```bash
cd <openclaw>
docker compose ps
curl -sS http://127.0.0.1:18789/healthz
```

Token mismatch:

- The OpenClaw Gateway token must match `auth.token` in
  `~/.vifu/providers.json`.
- The value should be the token itself, not an environment assignment.

Wrong URL:

- Use `http://host.docker.internal:18789` when Vifu runs in Docker and OpenClaw
  is published on the host.
- Use the real Gateway URL if OpenClaw runs on another machine.
