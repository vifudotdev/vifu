# Positioning And Related Projects

Vifu is infrastructure for putting agents inside products and operating that
product boundary. It combines an embeddable runtime with stable named
endpoints, versioned manifests, sessions, a reconnecting Gateway, and one
operations Console.

The product supplies domain state, user experience, safety policy, and allowed
actions. A provider supplies inference or device capabilities. Vifu keeps the
contract between them stable and observable.

## Adjacent approaches

| Approach | Representative projects | Primary optimization | Vifu's focus |
| --- | --- | --- | --- |
| Personal-assistant and OpenClaw rewrites | [OpenClaw](https://github.com/openclaw/openclaw), [PicoClaw](https://github.com/sipeed/picoclaw), [ZeroClaw](https://github.com/openagen/zeroclaw), [MimiClaw](https://github.com/memovai/mimiclaw) | A complete assistant/agent loop packaged for a language or hardware class | A library and operating contract embedded in an independently designed product |
| Device-oriented agent runtimes | [TuyaOpenClaw](https://github.com/tuya/TuyaOpenClaw), [ESPAgent](https://github.com/cube1345/ESPAgent), and other ESP-class runtimes | Tool use and agent behavior on SoCs, microcontrollers, or robots | Product-owned state/actions plus portable endpoints, releases, Gateway transport, and traces |
| Embeddable local agent loops | [EdgeChain](https://docs.rs/crate/edgechain-core/0.1.0) and native in-process runtimes such as Corvus | An agent loop and tool registry linked into mobile, game, robotics, or edge software | Stable product endpoints, versioned configuration, sessions, optional remote operation, and provider interchange |
| Local inference bindings | [llama.cpp](https://github.com/ggml-org/llama.cpp), [llama.rn](https://github.com/mybigday/llama.rn) | Efficient model execution and native/mobile bindings | Provider-neutral agent identity, sessions, invocation lifecycle, and operations around inference |
| Agent frameworks | [Google ADK](https://github.com/google/adk-python), [LangGraph](https://github.com/langchain-ai/langgraph), [Cloudflare Agents SDK](https://github.com/cloudflare/agents) | Agent/workflow definition, orchestration, or hosted durable instances | Stable project capabilities that can route to framework agents or local providers |
| AI-PC systems | [AMD GAIA](https://github.com/amd/gaia) | An integrated local-agent experience for AI PCs | An embeddable product SDK plus the same optional Server/Gateway operating path |
| Trusted edge-agent research | ARM CCA/TEE systems such as [AgenTEE](https://arxiv.org/abs/2604.18231) | Isolating inference, runtime, and applications inside trusted execution environments | Runtime and transport boundaries that can adopt platform isolation as a host concern |

The categories overlap. A small-device runtime can be a Vifu provider; a
framework agent can sit behind a Vifu endpoint; llama.cpp is the inference
backend used by Vifu's current local provider.

## Current differentiators

- **Product-shaped API:** applications invoke named capabilities, not a
  framework graph or assistant channel.
- **One local-to-operated path:** the same Runtime configured from Project Settings can be
  called in process and exposed through `EmbeddedRuntimeGateway`.
- **Thin host integration:** Rust and Swift call the Runtime directly; engine
  adapters carry `vifu.runtime-bridge/1` frames.
- **Constrained-resource controls:** strict structured output, bounded
  context/output, CPU/Metal controls, and traceable endpoint smoke tests.
- **Clear concurrency semantics:** registered Agent count, resident model
  count, and active invocation concurrency are separate measurements.

## Current limits

The published integration matrix is intentionally literal. Rust and the Apple
Swift package are supported. The Apple-hosted Godot bridge and Kotlin/Android
bindings are experimental. Generic Godot and managed-language adapters remain
future integration work and are not described as current support.

Any latency, memory, binary-size, or task-success comparison needs a committed
workload, pinned artifacts, and raw traces before it is published as evidence.
