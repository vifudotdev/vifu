# Agent Providers

Vifu Agent Gateway connects Vifu to external and in-process Agent Providers.
External providers own their model credentials, workspaces, tools, memory, and
agent definitions. In-process providers keep their model and resource settings
in the local Provider registry. Vifu makes the resulting capabilities
manageable through Projects, endpoints, keys, status, and logs.

Provider registrations use the same Vifu Server, Dashboard, database, and Agent
Gateway deployment. You can add, disable, replace, or delete a registration
without changing those services.

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

`key` is the stable Vifu-side provider identifier and `type` selects the
adapter. Network providers use `url` and may use `auth.token`. In-process
providers use `config` for device-local settings such as a model path. Create
`providers.json` before connecting a provider to a Vifu project.

| Provider | Status | Guide |
| --- | --- | --- |
| OpenClaw | Supported local provider | [OpenClaw](openclaw/) |
| llama.cpp GGUF | Supported in-process provider | [Local llama](llama/) |
| Whisper GGML | Supported in-process provider | [Local Whisper](local-whisper/) |
