# Examples

`examples/` contains runnable end-to-end applications. Reusable host, engine,
and protocol adapters belong in [`integrations/`](../integrations/README.md),
so an example may depend on an integration without copying its implementation.

| Example | Purpose | External runtime |
| --- | --- | --- |
| [`ios-embedding`](ios-embedding/) | Native SwiftUI app with an embedded Runtime, local model, Gateway pairing, and monitoring | None |
| [`godot-ios-embedding`](godot-ios-embedding/) | iOS host that connects the embedded Runtime to a Godot stage | libgodot and SwiftGodotKit |

Start with `ios-embedding` when validating the Vifu Runtime and mobile Gateway.
Use `godot-ios-embedding` when validating the additional engine lifecycle and
Runtime Bridge boundary.
