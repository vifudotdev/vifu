# Agent Providers

Vifu Agent Gateway connects Vifu to external agent providers. Providers own
their own model credentials, workspaces, tools, memory, and agent definitions.
Vifu discovers what a provider exposes, then makes those agents manageable
through Projects, endpoints, keys, status, and logs.

Provider integrations are independent from the Vifu self-host stack. You can
add, disable, replace, or delete provider registrations without changing the
server, dashboard, database, or Agent Gateway services.

Vifu reads provider registrations from the user configuration file at
`~/.vifu/providers.json`. The registry is intentionally provider-neutral:

```json
{
  "providers": [
    {
      "key": "local-provider",
      "type": "openclaw",
      "url": "http://host.docker.internal:18789",
      "auth": {
        "token": "replace-with-provider-token"
      }
    }
  ]
}
```

`key` is the stable Vifu-side provider identifier. `type` selects the adapter.
`url` is the provider's API endpoint. `auth.token` is the provider credential
used by the adapter. If `providers.json` is absent, Vifu starts without
external providers.

| Provider | Status | Guide |
| --- | --- | --- |
| OpenClaw | Supported local provider | [OpenClaw](openclaw/) |
