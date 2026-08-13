# Vifu iOS Starter

Vifu iOS Starter runs a Vifu Runtime and a GGUF language model inside a native
SwiftUI application. It streams replies, speaks completed responses with the
system voice, and connects its embedded Gateway to Vifu for live tracing and
Runtime release delivery.

```text
iPhone or iPad
SwiftUI host
        |
        v
VifuEmbeddedRuntime -> local llama.cpp provider
        |
        v
embedded Gateway === HTTPS/WSS === Vifu Server === TUI and Dashboard
```

The Android and iOS Starters use the same pairing protocol and trace model.
The iOS implementation stores the machine identity, server certificate pin,
and server authorization in Keychain, then reconnects on later launches.

## Install and run

If a Vifu iOS Starter beta has been shared with your testing group, install it
through TestFlight, open it, and choose **Download model (469 MiB)**. You can
also import a GGUF already stored on the device. The source-build path below is
available when no beta is active.

The Starter downloads and verifies this default model:

```text
Repository: Qwen/Qwen2.5-0.5B-Instruct-GGUF
Revision:   df5bf01389a39c743ab467d734bf501681e041c5
File:       qwen2.5-0.5b-instruct-q4_k_m.gguf
Size:       491,400,032 bytes
SHA-256:    74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db
```

Downloaded and imported models remain in the app's Application Support
directory. Review a model repository and its license before redistributing
weights with another application.

## Pair and inspect

Connect the iPhone or iPad and developer computer to the same local network.
Start the downloaded Vifu binary with a Server address that the device can
reach:

```bash
./vifu \
  -c server.address=https://<computer-lan-address>:6790 \
  -c gateway.address=http://127.0.0.1:6790
```

On its first run, Vifu creates one permanent local App and connects its local
Gateway. Later starts reuse the same App.

Press `B`, open the project and its primary deployment in the Dashboard, then
choose **Pair gateway** and **Copy pairing code**. In the Starter, open the
Gateway sheet and paste the code. The Dashboard QR remains available for the
native scanner when the application-link bridge is configured.

The code contains Vifu's Server address, a one-time enrollment token, and the
local certificate pin. After successful pairing, agent activity appears in the
TUI and Dashboard. Making an Agent Profile version live creates an immutable
Runtime release for the primary deployment; the app applies it to its local
database and reports the active version.

The model executes inside the app. Closing the Vifu process stops live
monitoring and settings delivery. The embedded Runtime continues with its last
applied release and reconnects when the paired Server becomes available.

## Build from source

The source project is the advanced path. It requires Xcode 26 or newer, iOS 17
or newer, and an Apple Silicon iPhone or iPad for practical local inference.

Build the local Vifu Apple artifact from the repository root:

```bash
VIFU_APPLE_DIST_DIR="$PWD/Frameworks" \
  scripts/build-apple-package.sh
```

Open `examples/ios-starter/VifuIOSStarter.xcodeproj`, select the
`VifuIOSStarter` scheme, and run it on a physical device.

For an unsigned command-line build check:

```bash
xcodebuild \
  -project examples/ios-starter/VifuIOSStarter.xcodeproj \
  -scheme VifuIOSStarter \
  -destination 'generic/platform=iOS' \
  -configuration Debug \
  CODE_SIGNING_ALLOWED=NO \
  ONLY_ACTIVE_ARCH=YES \
  build
```

Apple device builds use the Vifu Swift package and an Apple signing team.
Release installation uses TestFlight because iOS device packages require Apple
distribution signing and provisioning. The GitHub release continues to carry
the reusable `VifuMobileFFI.xcframework.zip` for application developers.

## Scope

This Starter covers local model setup, streaming invocation, system speech,
durable Runtime releases, pairing, reconnect, monitoring, and profile delivery.
Godot embedding remains a separate integration surface so its engine and
resource requirements can be verified independently.
