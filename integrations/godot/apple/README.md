# VifuGodot for Apple Hosts

`VifuGodot` is the optional Swift package that connects a host-owned
`GodotInstance` to Vifu's transport-neutral Runtime Bridge. It moves complete
encoded frames between Godot's `GlobalState` node and
`VifuInProcessBridgeTransport`; Runtime routing and application message
handling remain outside this module.

The package is separate from the root `Vifu` package so applications that do
not embed Godot never resolve or link SwiftGodot. `VifuMobileFFI.xcframework`
also remains independent of Godot.

## Dependencies

The package manifest resolves the maintained Git dependencies by default:

| Package | Requirement |
| --- | --- |
| Vifu | `vifudotdev/vifu`, `main` while this integration is under development |
| SwiftGodotKit | `vifudotdev/SwiftGodotKit`, revision `f72ec6f0...` |
| SwiftGodot | `vifudotdev/SwiftGodot`, revision `6644df67...` |

These are the Vifu-compatible forks, including the restart-safe `dlopen` /
`dlclose` behavior used by the iOS hosts. They are not replaced by the upstream
SwiftGodot packages.

For development across unpublished commits, the manifest accepts explicit
local overrides:

```bash
VIFU_GODOT_VIFU_PATH="$PWD" \
VIFU_GODOT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
VIFU_GODOT_SWIFTGODOTKIT_PATH="$PWD/../libgodot/SwiftGodotKit" \
SWIFTGODOTKIT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
  swift test --package-path integrations/godot/apple
```

The current Xcode examples use those sibling source checkouts so a Vifu change
and its integration can be tested before either repository is pushed:

```text
workspace/
  vifu/
    integrations/godot/apple/    # VifuGodot package
  libgodot/
    SwiftGodot/
    SwiftGodotKit/
    build/libgodot.xcframework
```

Add `vifu/integrations/godot/apple` as a local package dependency in Xcode and
link its `VifuGodot` product to the application target:

```swift
import VifuRuntimeBridge
import VifuGodot

let transport = VifuInProcessBridgeTransport()
let bridge = VifuGodotInProcessBridge(transport: transport)
try await bridge.connect(to: startedGodotInstance)
```

The application owns creation, frame iteration, restart, and destruction of
the Godot instance. Call `disconnect()` before replacing or destroying that
instance.

## Compatibility

This checkout is validated with:

| Component | Supported baseline |
| --- | --- |
| Swift tools | 5.9 or newer |
| Apple deployment | iOS 17+, macOS 14+ |
| Godot/libgodot | 4.5.x; current workspace uses 4.5.1 |
| SwiftGodot and SwiftGodotKit | compatible `libgodot_damon_45` checkouts |

SwiftGodot, SwiftGodotKit, and libgodot must describe the same Godot API. Treat
them as one compatibility set when updating. SwiftGodotKit remains code-only
because it loads and unloads `libgodot.xcframework` at runtime. Local workspaces
can supply that framework directly; the public VifuGodot distribution owns its
prebuilt release artifact.

### Prebuilt libgodot releases

Vifu publishes the compatible Apple runtime as immutable assets on a dedicated
release tag such as `libgodot-4.5.1-vifu.1`:

- `ios_libgodot.xcframework.zip`
- `mac_libgodot.xcframework.zip`
- their SwiftPM SHA-256 checksum files
- Godot's license and copyright notices

The tag is independent from normal Vifu Runtime versions because this binary
changes much less frequently. The manual **Release Vifu libgodot binaries**
workflow builds `template_release` device, simulator, and macOS slices from the
maintained `chenyanming/libgodot` fork. It never overwrites an existing artifact
tag.

After the first release exists, its immutable URLs and checksums belong in this
package as platform-conditional `binaryTarget` entries. Selecting the bundled
VifuGodot product then downloads the matching runtime instead of compiling
Godot in the consuming application. Do not commit placeholder checksums: first
produce the release assets, then pin the exact values reported by the workflow.
The initial sequence is therefore:

1. push the release workflow and the matching libgodot fork commit;
2. dispatch it with `publish=true` to build and publish the immutable assets;
3. copy the reported checksums into `Package.swift` and verify a clean remote
   SwiftPM resolution;
4. publish the VifuGodot source package tag.

SwiftPM does not support selecting a nested package by subdirectory from a Git
URL. The dependencies above are remote, but installing `VifuGodot` itself with
one Git URL requires publishing this directory as its own repository and tag.

## Verify

From the Vifu repository root:

```bash
VIFU_GODOT_VIFU_PATH="$PWD" \
VIFU_GODOT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
VIFU_GODOT_SWIFTGODOTKIT_PATH="$PWD/../libgodot/SwiftGodotKit" \
SWIFTGODOTKIT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
  swift test --package-path integrations/godot/apple
```

The focused tests use a fake Godot host to verify bidirectional frames,
reconnection, and disconnection. Build the
[`godot-ios-embedding`](../../../examples/godot-ios-embedding/) example on a
physical device to validate the libgodot runtime and rendering boundary.
