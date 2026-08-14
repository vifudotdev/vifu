# Examples

`examples/` contains runnable applications. Reusable host, engine, and protocol
adapters belong in [`integrations/`](../integrations/README.md).

## Mobile starters

| Starter | Installation path | Source project |
| --- | --- | --- |
| [Android Starter](android-starter/) | Install the optimized or baseline APK from a Vifu release | Kotlin app with modular Vifu AARs |
| [iOS Starter](ios-starter/) | Install a shared TestFlight beta, when available | Native SwiftUI app with the Vifu Swift package |
| [Godot iOS Starter](godot-ios-starter/) | Build with Xcode for a physical device | SwiftUI host with VifuGodot, SwiftGodotKit, and libgodot |

Start with the Android or iOS Starter to validate local inference, Gateway
pairing, reconnect, and tracing. Use the Godot iOS Starter when the application
also needs the engine lifecycle and Runtime Bridge.

## Performance examples

| Example | Purpose | External runtime |
| --- | --- | --- |
| [`macbook-agent-swarm`](macbook-agent-swarm/) | Compare local-provider settings under concurrent logical-agent load | Vifu CLI and a local Provider |

## Language starters

| Example | Embedded Runtime | What it proves |
| --- | --- | --- |
| [`python-starter`](python-starter/) | Rust through UniFFI | Automatic local connection, Python Agent invocation, and trace upload |
| [`typescript-starter`](typescript-starter/) | Rust through WebAssembly | TypeScript Provider registration, invocation, and local trace |

## Framework and model adapters

| Example | Role of Vifu |
| --- | --- |
| [`google-adk-python`](google-adk-python/) | Expose a Vifu endpoint as an ADK Python function tool |
| [`google-adk-typescript`](google-adk-typescript/) | Expose a Vifu endpoint as an ADK TypeScript `FunctionTool` |
| [`foundry-local-python`](foundry-local-python/) | Run self-managed Foundry Local chat with session state and Dashboard traces |
| [`foundry-local-typescript`](foundry-local-typescript/) | Wrap Foundry Local native streaming chat as a Vifu Provider |

Each framework adapter has a focused fake-provider test. This verifies the
Vifu boundary independently of model downloads and third-party credentials.
Use the [language tutorials](../docs/get-started/README.md) for the common
Runtime, Gateway, and tracing workflow.
