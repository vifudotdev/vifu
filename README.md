# Vifu

![Vifu](npm-packages/dashboard/public/brand/vifu-lockup.png)

[![Crates.io](https://img.shields.io/crates/v/vifu.svg)](https://crates.io/crates/vifu)
[![Runtime API](https://docs.rs/vifu-runtime/badge.svg)](https://docs.rs/vifu-runtime)
[![CI](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml/badge.svg)](https://github.com/vifudotdev/vifu/actions/workflows/ci.yml)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.com/invite/VdqqFwJbNE)

**Agent runtime in Rust.**

## Embed and operate agents inside products.

Vifu gives an application one stable contract for local and remote agents. The
product owns its state, interface, safety rules, and allowed actions. Vifu owns
provider connections, agent identity, versioned configuration, sessions,
named endpoints, cancellation, and traces.

Use the Runtime as a Rust library, ship it in an Apple application, or connect
the same embedded agents to Vifu Server and its operations Console.

## The product boundary

| Your product owns | Vifu owns | Providers own |
| --- | --- | --- |
| Game state, UI, action allowlists, domain policy | Agent registry, stable endpoints, sessions, releases, Gateway transport, traces | Model inference, hosted APIs, device capabilities |

This boundary is the reason to embed Vifu: agent implementations can change
while application code continues to invoke a named product capability.

```text
product code -> named Vifu endpoint -> agent -> local or remote provider
```

## Embed the Runtime

Add the library:

```toml
[dependencies]
vifu-runtime = "0.1"
```

Register a provider once, apply the product manifest, and invoke a named
endpoint from an application session:

```rust
use std::sync::Arc;
use vifu_runtime::prelude::*;

let mut provider = HttpCapabilityProvider::new(
    "models",
    "https://provider.example.com/v1",
    Some(std::env::var("MODEL_API_TOKEN")?),
)?;
provider.add_route(
    "chat",
    HttpCapabilityRoute::OpenAiChat {
        model: "model-name".into(),
        persona: json!({ "instructions": "Guide the player concisely." }),
    },
)?;

let runtime = VifuRuntime::new("my-product")?;
runtime.register_provider("models", Arc::new(provider))?;
runtime.apply_manifest(RuntimeManifest::from_json(include_bytes!("vifu.json"))?)?;

let output = runtime
    .session("player-42")?
    .invoke(InvocationInput::json(
        "town-guide",
        json!({
            "messages": [{ "role": "user", "content": "Open the north gate" }]
        }),
    ))
    .await?;
```

`vifu.json` is portable product configuration. Credentials and device paths
stay in the host:

```json
{
  "schemaVersion": 1,
  "projectId": "my-product",
  "providers": [{
    "id": "models",
    "providerType": "openai-compatible",
    "capabilities": ["chat"]
  }],
  "agents": [{
    "id": "guide",
    "name": "Town Guide",
    "provider": "models",
    "capabilities": ["chat"]
  }],
  "endpoints": [{
    "name": "town-guide",
    "agent": "guide",
    "capability": "chat",
    "timeoutMs": 30000
  }]
}
```

The same Runtime has async, start/poll/cancel, ordered output events, snapshots,
and the transport-neutral `vifu.runtime-bridge/1` protocol. See
[Embed the Runtime](docs/runtime-embedding.md).

## Run local providers

The `vifu` binary includes the in-process llama.cpp Provider so installed
release builds can run local GGUF models directly. Configure its GGUF path and
resource limits in `~/.vifu/providers.json`, then start Vifu normally:

```bash
mkdir -p ~/.vifu/models
cp providers/llama/providers.example.json ~/.vifu/providers.json
vifu
```

When a registry already contains Providers, add the `llama` entry instead of
replacing the file. Relative model paths resolve from the registry directory.
See [Local llama Provider](providers/llama/) for every supported setting.

The same registry also supports Local Whisper for speech-to-text. Add a
`local-whisper` entry with a model file from `~/.vifu/models`; projects and
endpoints bind to the resulting `transcription` capability instead of storing
device-local model paths in Server settings.

The Provider keeps the model resident, streams text fragments into the Runtime,
reports token counts, and supports strict JSON Schema output. Apple builds
enable Metal; `gpuLayers: 0` provides a CPU control path.

Requests accept both Vifu and OpenAI-compatible structured-output fields:

```json
{
  "messages": [{ "role": "user", "content": "Choose an action" }],
  "max_tokens": 64,
  "temperature": 0,
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "action",
      "strict": true,
      "schema": {
        "type": "object",
        "properties": { "action": { "type": "string" } },
        "required": ["action"],
        "additionalProperties": false
      }
    }
  }
}
```

Vifu validates the schema before inference and returns a provider error when a
generation ends before producing valid JSON. Responses retain native
`text`/`message` fields and also expose OpenAI-compatible `choices` and `usage`.

## Connect embedded agents to the Console

The reusable `EmbeddedRuntimeGateway` runs beside any manifest-configured
`VifuRuntime`:

```rust
use vifu_gateway::embedded::{
    EmbeddedRuntimeGateway,
    EmbeddedRuntimeGatewayConfig,
};
use vifu_gateway::identity::MachineIdentity;

let gateway = EmbeddedRuntimeGateway::new(
    runtime.clone(),
    EmbeddedRuntimeGatewayConfig::new(
        "https://runtime.example.com",
        "runtime.sqlite",
    )
    .with_dashboard_url("https://dashboard.example.com"),
)?;
let identity = MachineIdentity::from_encoded_private_key(&machine_private_key)?;
gateway.start(identity, device_token, enrollment_token)?;
```

The Machine private key is stable host identity; the Server returns a
server-scoped Device Token after authorization. One authenticated Gateway
connection publishes the manifest's agents, carries
remote invocations, synchronizes releases, and uploads safe trace summaries.
The Console operates projects, deployments, provider bindings, keys, connected
Gateways, available agents, and traces.

## Supported integration surfaces

| Surface | Status | Integration contract |
| --- | --- | --- |
| Rust | Supported | `vifu-runtime`, `vifu-gateway`, and provider traits |
| Swift on iOS/macOS | Supported | Swift Package plus versioned XCFramework and UniFFI API |
| Godot in an Apple host | Experimental | Thin signal/frame adapter over Runtime Bridge |
| Kotlin/Android | Experimental | Generated UniFFI bindings and native callback providers |

Runtime Bridge keeps engine adapters thin: they move complete JSON frames while
Vifu retains endpoint, session, event, and cancellation behavior. Additional
managed-language and engine bindings can implement the same protocol.

## Run Vifu Server and Console

The `vifu` package runs Server and Agent Gateway roles from one binary:

```bash
cargo install vifu
vifu
```

For the complete self-hosted stack:

```bash
cp .env.example .env
docker compose up -d
```

Open `http://localhost:6791`. Applications call a project-scoped,
OpenAI-compatible endpoint:

```http
POST http://localhost:6790/my-product/v1/chat/completions
Authorization: Bearer vifu_pk_...
Content-Type: application/json

{
  "model": "town-guide",
  "messages": [{ "role": "user", "content": "Open the north gate" }]
}
```

See [Self-host Vifu](docs/self-hosting.md) for enrollment, network boundaries,
and upgrades.

## Architecture

```text
Embedded product
  UI / game loop -> VifuRuntime -> local provider
                       |
                       +-> optional EmbeddedRuntimeGateway
                                      |
Application -> project endpoint -> Vifu Server -> Gateway -> provider agents
                                      |
Console ------------------------------+ projects / releases / traces
```

| Component | Responsibility |
| --- | --- |
| Embedded Runtime | One product's providers, agents, endpoints, sessions, state, effects, and invocation lifecycle |
| Vifu Server | Multi-project HTTP/WebSocket access, keys, deployment state, routing, and traces |
| Agent Gateway | Authenticated, multiplexed transport between provider resources and Server |
| Operations Console | One operating surface for projects, deployments, agents, Gateways, keys, and traces |

## Documentation

- [Documentation index](docs/README.md)
- [Install from source](docs/install.md)
- [Embed the Runtime](docs/runtime-embedding.md)
- [Self-host Vifu](docs/self-hosting.md)
- [Positioning and related projects](docs/comparison.md)
- [Provider integrations](providers/README.md)
- [Build and test](BUILD.md)

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Report
security issues through [SECURITY.md](SECURITY.md).

Vifu is licensed under [Apache-2.0](LICENSE). The license does not grant rights
to the Vifu name and logos; see [TRADEMARKS.md](TRADEMARKS.md).
