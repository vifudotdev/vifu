# Build A Godot Agent App With Vifu

VifuGodot connects a Godot scene to the embedded Vifu Runtime through a
Swift-hosted in-process bridge. The scene remains the product UI. Vifu handles
Agent invocation, Gateway transport, and tracing.

## 1. Add VifuGodot

In Xcode, add `https://github.com/vifudotdev/VifuGodot` and link the
`VifuGodot` product. The package resolves the matching Vifu Runtime,
SwiftGodot, SwiftGodotKit, and libgodot compatibility set.

## 2. Connect The Bridge

```swift
import VifuRuntimeBridge
import VifuGodot

let transport = VifuInProcessBridgeTransport()
let bridge = VifuGodotInProcessBridge(transport: transport)
try await bridge.connect(to: startedGodotInstance)
```

The host owns Godot creation, frames, restart, and destruction. Disconnect the
bridge before replacing the instance.

## 3. Invoke From Godot

Godot sends a `runtime.invoke` bridge frame with an endpoint, session ID, JSON
input, and metadata. It then consumes output delta and terminal frames. Keep
the character, scene, and gameplay as the primary interface; open pairing or
lesson controls only in response to in-world actions.

The complete frame flow is in the
[`godot-ios-starter`](../../examples/godot-ios-starter/). The reusable package
contract is in the [VifuGodot guide](../../integrations/godot/apple/README.md).

## 4. Pair And Inspect

The Swift host pairs the same embedded Runtime through
`VifuEmbeddedGateway`. Provider stages and device metadata then appear in the
Vifu Dashboard. The game continues to invoke its local Runtime directly; the
Gateway adds remote routing and monitoring.
