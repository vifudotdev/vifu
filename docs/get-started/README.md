# Build With Vifu

Vifu embeds a Rust Agent Runtime in your application. Your code registers a
Provider, an Agent, and a stable endpoint. The Runtime connects to a Vifu
Server so the Dashboard can show device status and inference traces.

Choose the language used by your application:

| Language or host | Runtime form | Tutorial | Runnable proof |
| --- | --- | --- | --- |
| Python | Rust through UniFFI | [Python](python.md) | [`python-starter`](../../examples/python-starter/) |
| TypeScript | Rust compiled to WebAssembly for Node.js | [TypeScript](typescript.md) | [`typescript-starter`](../../examples/typescript-starter/) |
| Swift | UniFFI and XCFramework | [Swift](swift.md) | [`ios-starter`](../../examples/ios-starter/) |
| Kotlin | UniFFI/JNI and modular AARs | [Kotlin](kotlin.md) | [`android-starter`](../../examples/android-starter/) |
| Godot on Apple platforms | Godot bridge and Swift host | [Godot](godot.md) | [`godot-ios-starter`](../../examples/godot-ios-starter/) |
| Rust | Native crate | [Rust](rust.md) | [`vifu-runtime` public API tests](../../crates/vifu-runtime/tests/public_api.rs) |

## The Common Development Loop

Every SDK follows the same six tasks:

1. Create one Runtime for the application.
2. Register a Provider function or model adapter.
3. Register an Agent and give it a stable endpoint.
4. Invoke the endpoint from product code.
5. Report model stages such as `load`, `prefill`, `first_token`, and `decode`.
6. Connect the Runtime to Vifu for remote calls and Dashboard traces.

The Runtime is the in-process application API. Gateway is the secure Server
connection for that Runtime. Python joins the local App automatically. Remote
devices use an App ID or explicit enrollment.

## Choose What Vifu Records

Performance telemetry is content-private by default. It includes identity,
stage timing, token counts when supplied, status, and bounded errors. Prompt
and output content is sent only when the host enables trace content after an
explicit consent decision.

Use the [observability guide](../observability.md) to choose stage metadata and
verify the data in the Dashboard. Use the [integration guides](../integrations/README.md)
when another agent framework or model runtime owns part of the application.
