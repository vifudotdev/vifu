# Vifu iOS Embedding

This example runs a Vifu Runtime and a GGUF language model inside a native
SwiftUI application. It streams replies, speaks completed responses with the
system voice, and optionally connects its embedded Gateway to Vifu Server for
live monitoring and Runtime release delivery.

```text
iPhone
SwiftUI host
        |
        v
VifuEmbeddedRuntime -> local llama.cpp provider
        |
        v
embedded Gateway === HTTPS/WSS === Vifu Server === TUI and Dashboard
```

The app stores one Vifu Server URL. HTTP control calls and the Agent Gateway
WebSocket are derived from that address. Pairing pins Vifu's generated local
certificate; deployments using a publicly trusted certificate use the iOS
system trust store.

The Runtime and its active release remain in the app's Application Support
database. Closing Vifu Server stops monitoring and settings delivery, but the
embedded Runtime continues using its last applied release. The embedded Gateway
reconnects when the paired server becomes available again.

The first launch offers two model setup paths:

- download the pinned Qwen3 1.7B Q4_K_M model and verify its SHA-256 digest;
- import a `.gguf` model already stored on the device.

Downloaded and imported models remain in the app's Application Support
directory. Model weights are not stored in this repository.

## Requirements

- Xcode 26 or newer
- iOS 17 or newer
- an Apple Silicon iPhone or iPad for practical local inference

The example has no external game-engine checkout or generated resource pack.
Its presentation is native SwiftUI so the Runtime, model, pairing, monitoring,
and profile-delivery path can be reproduced independently.

## Build and run

From the repository root, build the local Vifu Apple artifact used by the Swift
package:

```bash
VIFU_APPLE_DIST_DIR="$PWD/Frameworks" \
  scripts/build-apple-package.sh
```

Open `examples/ios-embedding/IOSEmbeddingDemo.xcodeproj`, select the
`IOSEmbeddingDemo` scheme, and run it on a physical iOS device.

For an unsigned command-line build check:

```bash
xcodebuild \
  -project examples/ios-embedding/IOSEmbeddingDemo.xcodeproj \
  -scheme IOSEmbeddingDemo \
  -destination 'generic/platform=iOS' \
  -configuration Debug \
  CODE_SIGNING_ALLOWED=NO \
  ONLY_ACTIVE_ARCH=YES \
  build
```

## Pair with Vifu Server

Copy the server-only profile and replace `your-macbook.local` with a hostname
or address that the iPhone can reach:

```bash
mkdir -p ~/.vifu
cp examples/ios-embedding/vifu-server.example.toml \
  ~/.vifu/ios-embedding.toml
```

For self-hosted mode, create persistent independent secrets:

```bash
export VIFU_DEMO_SECRETS="$HOME/.vifu/ios-embedding-secrets"
mkdir -p "$VIFU_DEMO_SECRETS"
chmod 700 "$VIFU_DEMO_SECRETS"
openssl rand -hex 32 > "$VIFU_DEMO_SECRETS/admin"
openssl rand -hex 32 > "$VIFU_DEMO_SECRETS/gateway"
openssl rand -hex 32 > "$VIFU_DEMO_SECRETS/api-key-pepper"
openssl rand -hex 32 > "$VIFU_DEMO_SECRETS/provider-secret"
chmod 600 "$VIFU_DEMO_SECRETS"/*
export VIFU_DEPLOYMENT_MODE=self-hosted
export VIFU_ADMIN_KEY_FILE="$VIFU_DEMO_SECRETS/admin"
export VIFU_AGENT_GATEWAY_BOOTSTRAP_TOKEN_FILE="$VIFU_DEMO_SECRETS/gateway"
export VIFU_API_KEY_PEPPER_FILE="$VIFU_DEMO_SECRETS/api-key-pepper"
export VIFU_PROVIDER_SECRET_KEY_FILE="$VIFU_DEMO_SECRETS/provider-secret"
vifu --profile ios-embedding
```

The one `server.address` value is the origin used by the app, Dashboard,
enrollment API, Runtime configuration API, telemetry, and Agent Gateway
WebSocket. Vifu generates and retains a pairing certificate for a local HTTPS
address. A reverse proxy, tunnel, or hosted ingress can provide the same origin
with a publicly trusted certificate.

In the Dashboard:

1. Create or open the project with slug `ios-embedding`.
2. Open its primary deployment and choose **Pair gateway**.
3. Open the Gateway sheet in the iOS app and scan the pairing code.

The project slug must match the embedded Runtime project ID. After successful
pairing, the app keeps its machine identity, server binding, optional
certificate pin, and server authorization in Keychain and reconnects on later
launches.

Press `B` in the Vifu TUI to open the Dashboard. Agent activity from the iPhone
appears in the live TUI. Making an Agent Profile version live creates an
immutable Runtime release for the primary deployment; the app applies it to its
local database and reports the applied version.

## Model

The default setup downloads and verifies:

```text
Repository: ggml-org/Qwen3-1.7B-GGUF
Revision:   daeb8e2d528a760970442092f6bf1e55c3b659eb
File:       Qwen3-1.7B-Q4_K_M.gguf
Size:       1,282,439,264 bytes
SHA-256:    d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5
```

Review the model repository and license before redistributing a downloaded
model with an application.

## Scope

This example validates the native iOS Runtime path: local model loading,
streaming invocation, durable Runtime releases, pairing, reconnect, monitoring,
and profile delivery. Godot embedding is intentionally kept in a separate
integration surface so it can be reviewed and reproduced with its own engine
and resource requirements.
