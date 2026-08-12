# Vifu Mobile Starter

Vifu Mobile Starter provides the same product loop on Android and iOS: run a
local model inside the app, pair its embedded Agent Runtime with Vifu, and
inspect the invocation in the TUI and Dashboard.

## Choose a platform

| Platform | Installation path | Source project |
| --- | --- | --- |
| Android | Install the APK from the Vifu GitHub release | [Android Starter](../android-starter/README.md) |
| iPhone and iPad | Install a shared TestFlight beta, when available | [iOS Starter](../ios-embedding/README.md) |

Android release assets include optimized and baseline ARM64 builds. The iOS
Starter uses the same embedded Runtime, local llama.cpp provider, Gateway
pairing protocol, durable device identity, and trace model through Vifu's Swift
package.

After installation, both apps can download the verified Qwen2.5 0.5B GGUF or
import another model. Pairing binds the app to a local Vifu Server with a
one-time enrollment and certificate pin. Later launches restore the saved
device authorization and reconnect automatically.

Application developers can start from the platform source projects after they
have verified the release app loop.
