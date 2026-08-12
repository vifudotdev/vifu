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
