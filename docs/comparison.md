# Positioning and Related Projects

Vifu puts agents inside products. It operates the boundary between a product
and its AI providers.

Vifu includes an embeddable Runtime, named endpoints, versioned manifests,
sessions, a Gateway that reconnects, and one operations Console.

The product owns its domain state, user experience, safety policy, and allowed
actions. A provider supplies inference or device capabilities. Vifu keeps the
contract between them stable and observable.

## Adjacent Approaches

| Approach | Representative projects | Primary focus | Vifu focus |
| --- | --- | --- | --- |
| Personal assistants | [OpenClaw](https://github.com/openclaw/openclaw), [PicoClaw](https://github.com/sipeed/picoclaw), [ZeroClaw](https://github.com/openagen/zeroclaw), and [MimiClaw](https://github.com/memovai/mimiclaw) | A complete assistant loop for one language or hardware class | A runtime contract inside a separate product |
| Device agent runtimes | [TuyaOpenClaw](https://github.com/tuya/TuyaOpenClaw), [ESPAgent](https://github.com/cube1345/ESPAgent), and other runtimes for small devices | Tool use and agent behavior on devices, robots, or microcontrollers | Product-owned state and actions with portable endpoints, releases, transport, and traces |
| Embedded agent loops | [EdgeChain](https://docs.rs/crate/edgechain-core/0.1.0) and native in-process runtimes such as Corvus | An agent loop and tool registry inside an application | Stable endpoints, versioned configuration, sessions, remote operation, and provider interchange |
| Local inference bindings | [llama.cpp](https://github.com/ggml-org/llama.cpp) and [llama.rn](https://github.com/mybigday/llama.rn) | Model inference and native bindings | Agent identity, sessions, invocation control, and operations around inference |
| On-device AI runtimes | [Google LiteRT and LiteRT-LM](https://developers.google.com/edge/litert/overview) and [Microsoft Foundry Local](https://github.com/microsoft/Foundry-Local) | Model acquisition, hardware acceleration, and device inference APIs | A stable endpoint contract for products above the inference layer |
| Agent frameworks | [Google ADK](https://github.com/google/adk-python), [LangGraph](https://github.com/langchain-ai/langgraph), and [Cloudflare Agents SDK](https://github.com/cloudflare/agents) | Agent definitions, workflows, orchestration, or durable hosted agents | Stable product capabilities that use framework agents or local providers |
| AI PC systems | [AMD GAIA](https://github.com/amd/gaia) | An integrated local agent for AI PCs | An embedded product SDK with optional Server and Gateway operation |
| Trusted edge research | Trusted execution systems such as [AgenTEE](https://arxiv.org/abs/2604.18231) | Isolation for inference, runtimes, and applications | Runtime and transport boundaries that can use host isolation |

These categories overlap. A runtime for a small device can operate as a Vifu
provider. A framework agent can operate behind a Vifu endpoint. The current
local provider in Vifu uses llama.cpp as its inference backend.

## LiteRT, Foundry Local, and Vifu

**Short answer:** LiteRT runs models on devices. Foundry Local puts local models
inside desktop applications. Vifu connects products to Agents and manages their
endpoints, sessions, access, routes, and traces.

| Product | Main job | Best fit |
| --- | --- | --- |
| [Google LiteRT and LiteRT-LM](https://developers.google.com/edge/litert/overview) | Run models with the CPU, GPU, or NPU of a device | Mobile, Web, desktop, and embedded applications |
| [Microsoft Foundry Local](https://github.com/microsoft/Foundry-Local) | Download, select, and run local models inside an application | Windows, macOS, and Linux applications |
| Vifu | Give products stable Agent endpoints and operate them | Products that need sessions, routing, keys, Gateway transport, and traces |

LiteRT and Foundry Local solve the model problem. They select a model and run it
on available hardware.

Vifu solves the product problem. It connects a product to an Agent through a
stable endpoint. The Agent can use a local or remote provider.

The products overlap at the inference API. All three can give an application
access to AI. LiteRT-LM and Foundry Local also support tool calling.

Local inference is not the main Vifu difference. Vifu adds the product contract
around inference:

- The endpoint stays the same after a provider or model change.
- Sessions and traces stay with the product.
- The Gateway connects local Agents to remote products.
- Keys control access to each endpoint.
- The product controls the actions that an Agent can use.

This structure puts LiteRT and Foundry Local below Vifu:

```text
product -> Vifu Agent endpoint -> local or remote provider
                                  |- llama.cpp
                                  |- LiteRT-LM
                                  |- Foundry Local
                                  `- cloud model
```

Vifu currently uses llama.cpp for its local provider. Vifu does not publish a
provider for LiteRT-LM or Foundry Local. The diagram shows the product boundary,
not current support.
