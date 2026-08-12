# Vifu Godot iOS Starter

This Starter adds a host-owned libgodot stage to the native Runtime path in the
[`iOS Starter`](../ios-starter/).

The directory boundary is intentional:

```text
integrations/godot/apple/
  complete VifuGodot Swift package for an Agent-aware libgodot host

examples/godot-ios-starter/
  iOS host, Godot stage, local model setup, and run instructions
```

The application owns Godot's lifecycle. Vifu does not create or destroy the
engine. `VifuRuntimeBridgeSession` carries the same Runtime Bridge frames used
by remote transports through `VifuInProcessBridgeTransport`.
The Xcode target imports the package product with `import VifuGodot`; it does
not compile integration source files directly.

## Requirements

- all requirements from the [`iOS Starter`](../ios-starter/README.md)
- Godot 4 with iOS export support

The Xcode target depends only on the `VifuGodot` product. SwiftPM resolves the
Vifu Agent Runtime, the maintained SwiftGodot and SwiftGodotKit forks, and the
platform-compatible prebuilt libgodot binary. No sibling libgodot checkout or
local Godot compilation is required to build the application.

The checked-in stage runs with a procedural placeholder. A local
`Godot/character.glb` or `Godot/character.vrm` can replace it. VRM import
requires a compatible VRM addon. Character files, third-party addons, imported
Godot data, generated packs, and model weights are intentionally not committed.

## Build

Export the checked-in stage:

```bash
mkdir -p examples/godot-ios-starter/VifuGodotIOSStarter/Resources
/Applications/Godot.app/Contents/MacOS/Godot --headless \
  --path "$PWD/examples/godot-ios-starter/Godot" \
  --export-pack Embedded \
  "$PWD/examples/godot-ios-starter/VifuGodotIOSStarter/Resources/godot-ios-starter.pck"
```

Open `examples/godot-ios-starter/VifuGodotIOSStarter.xcodeproj`, select the
`VifuGodotIOSStarter` scheme, and run it on a physical iOS device.

Run the bridge contract tests independently with:

```bash
swift test --package-path integrations/godot/apple
```

For an unsigned build check:

```bash
xcodebuild \
  -project examples/godot-ios-starter/VifuGodotIOSStarter.xcodeproj \
  -scheme VifuGodotIOSStarter \
  -destination 'generic/platform=iOS' \
  -configuration Debug \
  IPHONEOS_DEPLOYMENT_TARGET=17.0 \
  CODE_SIGNING_ALLOWED=NO \
  ONLY_ACTIVE_ARCH=YES \
  build
```

## Runtime and server

The embedded Runtime project ID is `godot-ios-starter`. Create a Dashboard
project with the same slug when testing profile delivery. Pairing, monitoring,
local model setup, and server configuration otherwise follow the
[`iOS Starter` instructions](../ios-starter/README.md#pair-and-inspect).

The Godot stage receives presentation events such as activity changes and can
send `runtime.*` requests through the in-process bridge. Runtime ownership,
release storage, model calls, and Gateway reconnect remain in the native host.
