# VifuGodot for Apple Hosts

`VifuGodot` is the complete Apple package for building an Agent-aware Godot
host. One product supplies the Vifu Agent Runtime, the in-process Godot bridge,
the maintained SwiftGodot host SDK, and the compatible prebuilt libgodot
binary. It moves complete encoded frames between Godot's `GlobalState` node
and `VifuInProcessBridgeTransport`; the host keeps ownership of the Godot
instance lifecycle.

The package is separate from the root `Vifu` package so applications that do
not embed Godot never resolve, download, or link Godot dependencies. Choose
`Vifu` for the Godot-free Runtime and `VifuGodot` for the complete Godot host.

## Dependencies

The package manifest resolves the maintained Git dependencies by default:

| Package | Requirement |
| --- | --- |
| Vifu Agent Runtime | `vifudotdev/vifu`, `main` while this integration is under development |
| SwiftGodotKit | `vifudotdev/SwiftGodotKit`, revision `b512ec68...` |
| SwiftGodot | `vifudotdev/SwiftGodot`, revision `971a8c0c...` |
| libgodot | Vifu release `libgodot-4.5.1-vifu.1`, selected for iOS or macOS by SwiftPM |

These are the Vifu-compatible forks and binary, including the tested instance
lifecycle used by the iOS hosts. They are not replaced by the upstream
SwiftGodot packages. Adding the `VifuGodot` product is sufficient; applications
do not add Vifu, SwiftGodot, SwiftGodotKit, or libgodot separately.

For development across unpublished commits, the manifest accepts explicit
local overrides. Extract the locally prepared release assets into this
package's ignored `.build` directory first:

```bash
mkdir -p integrations/godot/apple/.build/libgodot
unzip -q ../libgodot-release-source/build/vifu-release/libgodot-4.5.1-vifu.1/ios_libgodot.xcframework.zip \
  -d integrations/godot/apple/.build/libgodot
unzip -q ../libgodot-release-source/build/vifu-release/libgodot-4.5.1-vifu.1/mac_libgodot.xcframework.zip \
  -d integrations/godot/apple/.build/libgodot
```

Then resolve every unpublished source and binary from the local workspace:

```bash
VIFU_GODOT_VIFU_PATH="$PWD" \
VIFU_GODOT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
VIFU_GODOT_SWIFTGODOTKIT_PATH="$PWD/../libgodot/SwiftGodotKit" \
SWIFTGODOTKIT_SWIFTGODOT_PATH="$PWD/../libgodot/SwiftGodot" \
VIFU_GODOT_IOS_LIBGODOT_PATH=".build/libgodot/ios_libgodot.xcframework" \
VIFU_GODOT_MACOS_LIBGODOT_PATH=".build/libgodot/mac_libgodot.xcframework" \
  swift test --package-path integrations/godot/apple
```

This layout lets maintainers test a Vifu change and its integration before the
source commits or binary release are published:

```text
workspace/
  vifu/
    integrations/godot/apple/    # VifuGodot package
      .build/libgodot/            # ignored extracted release assets
  libgodot/
    SwiftGodot/
    SwiftGodotKit/
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
them as one compatibility set when updating. Local workspaces can override the
two binary targets with extracted XCFramework paths; the public VifuGodot
distribution resolves the immutable binaries automatically.

### Prebuilt libgodot releases

Vifu publishes the compatible Apple runtime as immutable assets on a dedicated
release tag such as `libgodot-4.5.1-vifu.1`:

- `ios_libgodot.xcframework.zip`
- `mac_libgodot.xcframework.zip`
- their SwiftPM SHA-256 checksum files
- Godot's license and copyright notices

The tag is independent from normal Vifu Runtime versions because this binary
changes much less frequently. A maintainer builds `template_release` device,
simulator, and macOS slices once on a known Apple development machine from an
exact `vifudotdev/libgodot` commit and its pinned `vifudotdev/godot` source.
The build command creates a draft Vifu release asset set; the manual **Release
Vifu libgodot binaries** workflow verifies that draft and optionally publishes
it. CI does not perform a cold Godot build, and neither path overwrites an
existing artifact tag.

The immutable URLs and checksums are pinned in `Package.swift` as
platform-conditional binary targets. Selecting VifuGodot downloads the matching
runtime instead of compiling Godot in the consuming application. The release
sequence is:

1. build and package the exact libgodot commit locally with
   `scripts/prepare-libgodot-apple-release.sh`;
2. inspect the output and create a draft with
   `scripts/create-libgodot-apple-draft.sh`;
3. dispatch the release workflow with the same commit and tag, using
   `publish=true` only after the draft is ready;
4. verify that the checksums pinned in `Package.swift` match the published
   assets and run a clean remote SwiftPM resolution;
5. publish the VifuGodot source package tag.

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
VIFU_GODOT_IOS_LIBGODOT_PATH=".build/libgodot/ios_libgodot.xcframework" \
VIFU_GODOT_MACOS_LIBGODOT_PATH=".build/libgodot/mac_libgodot.xcframework" \
  swift test --package-path integrations/godot/apple
```

The focused tests use a fake Godot host to verify bidirectional frames,
reconnection, and disconnection. Build the
[`godot-ios-embedding`](../../../examples/godot-ios-embedding/) example on a
physical device to validate the libgodot runtime and rendering boundary.
