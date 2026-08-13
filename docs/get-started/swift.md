# Build A Swift Agent With Vifu

Vifu supplies a Swift API generated from its Rust UniFFI contract. Use it in an
iOS 17+ or macOS 14+ application.

## 1. Add The Package

In Xcode, choose **File > Add Package Dependencies**, enter
`https://github.com/vifudotdev/vifu`, select a released version, and add the
`Vifu` product to the application target.

## 2. Open Persistent Runtime Storage

```swift
import Vifu

let databaseURL = applicationSupportURL.appendingPathComponent("runtime.sqlite")
let runtime = try VifuEmbeddedRuntime.open(
    projectId: "my-swift-app",
    databasePath: databaseURL.path
)
```

Use one stable project ID for one application identity. Store the database in
the protected application container.

## 3. Register A Provider And Agent

Implement `VifuAgentProvider` or `VifuStreamingAgentProvider`, register it with
the Runtime, then register its Agent and endpoint. For a local GGUF model, the
shorter built-in path is:

```swift
try runtime.registerLlamaProvider(
    providerId: "local-model",
    config: VifuLlamaProviderConfig(
        modelPath: modelURL.path,
        contextSize: 4_096,
        gpuLayers: UInt32.max,
        defaultMaxTokens: 220
    )
)
```

Apply a Runtime manifest that maps an Agent and stable endpoint to that
Provider. The iOS Starter includes a complete
[`StarterRuntime.json`](../../examples/ios-starter/VifuIOSStarter/StarterRuntime.json)
and bootstrap code.

## 4. Invoke From The App

Use `VifuRuntimeBridgeConnection` when UI code needs streamed events. Start a
`runtime.invoke` request, retain its returned handle, and consume output deltas
until the invocation completes. The complete implementation is in
[`EmbeddedRuntime.swift`](../../examples/ios-starter/VifuIOSStarter/EmbeddedRuntime.swift).

## 5. Pair With Vifu

Parse the camera result with `VifuGatewayPairingCode`. Store the machine
identity and Server authorization in Keychain. Start `VifuEmbeddedGateway`
with the Runtime and the one-time enrollment token. On later starts, omit the
consumed token and use the stored authorization.

The [Runtime embedding guide](../runtime-embedding.md#connect-an-embedded-runtime-to-vifu-server)
contains the complete Gateway code. The [iOS Starter](../../examples/ios-starter/)
is the runnable reference for camera pairing, reconnection, local inference,
and trace reporting.
