# Vifu Android

Vifu Android is split into three independently packaged native modules:

| Maven artifact | Native entry point | Purpose |
| --- | --- | --- |
| `dev.vifu:vifu-android-core` | `libvifu_mobile_ffi.so` | Agent Runtime, Gateway, state, traces, and provider lifecycle |
| `dev.vifu:vifu-android-llama` | `libvifu_llama_ffi.so` | ARM-optimized local llama.cpp chat provider |
| `dev.vifu:vifu-android-whisper` | `libvifu_whisper_ffi.so` | Local whisper.cpp transcription provider |

The llama and Whisper artifacts depend on Core. An app can package either
provider or both. Core loads a provider and its model only when the app calls
`attach` or `open`; closing the provider unregisters its endpoint and releases
its model resources after any in-flight invocation finishes.

The optimized llama artifact also packages llama.cpp's dynamic ARM CPU backend
libraries. It selects the best compatible backend on the phone and includes an
ARMv8 variant in that set. `dev.vifu:vifu-android-llama-baseline` is the
conservative, static ARMv8 alternative for device-specific compatibility
issues. Use one llama artifact per app.

## Add providers

For local chat:

```kotlin
dependencies {
    implementation("dev.vifu:vifu-android-llama:0.1.12")
}
```

For chat and transcription in the same app:

```kotlin
dependencies {
    implementation("dev.vifu:vifu-android-llama:0.1.12")
    implementation("dev.vifu:vifu-android-whisper:0.1.12")
}
```

Gradle resolves the shared Core dependency once. Applications that use only
the Runtime or Gateway can depend directly on `vifu-android-core`.

## Minimal chat API

The convenience API creates Core and attaches llama on demand:

```kotlin
val agent = VifuLlamaAgent.open(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath = modelFile.absolutePath),
)

agent.send("Hello").collect { token -> render(token) }
agent.close()
```

`open` runs on the IO dispatcher, loads the GGUF model, and registers the
provider, agent, and endpoint. `send` returns `Flow<String>`. Cancelling
collection cancels inference. Successful turns become conversation history;
failed or cancelled turns do not. Use `resetConversation()` to clear history.

## Share Core between llama and Whisper

Create one Core runtime when both models should be available to the same app
and Gateway connection:

```kotlin
val runtime = VifuAndroidRuntime.open(applicationContext)
val llama = VifuLlamaAgent.attach(
    runtime,
    VifuLlamaConfig(modelPath = chatModel.absolutePath),
)
val whisper = VifuWhisperAgent.attach(
    runtime,
    VifuWhisperConfig(modelPath = whisperModel.absolutePath),
)

val prompt = whisper.transcribe(wavBytes)
llama.send(prompt).collect { token -> render(token) }

whisper.close() // unloads only Whisper
llama.close()   // unloads only llama
runtime.close()
```

The native libraries remain mapped for the Android process lifetime, while
provider objects and model memory are released on `close`. A provider can be
attached again later.

## Gateway monitoring

Use `connect` for an app that supports Vifu pairing:

```kotlin
val agent = VifuLlamaAgent.connect(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath = modelFile.absolutePath),
    pairingCode = pairingCode, // Use this parameter only after a new scan.
    captureTraceContent = true, // Set this only after the user gives consent.
)
```

The SDK validates the pairing code. It also stores the Server binding after a
successful connection. On later app starts, omit `pairingCode`:

```kotlin
val agent = VifuLlamaAgent.connect(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath = modelFile.absolutePath),
    captureTraceContent = true,
)
```

The SDK restores the binding and reconnects with the stored device identity.
Call `VifuLlamaAgent.clearConnection(context)` to return to local-only use.

Use Core directly when one Gateway must contain multiple providers:

```kotlin
val runtime = VifuAndroidRuntime.open(
    context = applicationContext,
    connection = VifuConnectionConfig(
        serverUrl = "https://vifu.example:6790",
        appId = "vifu_app_...",
        serverCertificateDer = certificate,
    ),
)
val llama = VifuLlamaAgent.attach(runtime, VifuLlamaConfig(modelPath))
runtime.startGateway()
```

Core restarts an active Gateway connection when providers are attached or
detached so the remote manifest stays current. The machine private key and
device token are encrypted with Android Keystore. A one-time enrollment token
is sent only until the Server issues the device token. Later starts use the
same Server binding and stored device token. Build-time App ID configuration is
still available for managed application builds. App code does not manage
runtime IDs.

Android reports a readable Gateway name, application identity, device model,
OS version, and supported ABIs automatically. An embedded host can replace the
default mobile identity and add bounded product metadata:

```kotlin
val connection = pairing.connectionConfig().copy(
    gatewayName = "Kitchen light",
    gatewayKind = "light",
    gatewayAttributes = mapOf(
        "room" to "kitchen",
        "hardwareRevision" to "rev-b",
    ),
)
```

The Dashboard uses this identity on the Overview and Trace pages. The stable
Gateway ID remains available for diagnostics.

## Model-load diagnostics

Vifu reports the stage that failed instead of exposing llama.cpp's bare null
result. The message includes a stable code, safe model metadata, and registered
backend names; it never includes the model's full private path.

| Code | Meaning | App action |
| --- | --- | --- |
| `VIFU-LLAMA-BACKEND-001` | Native backend libraries are missing or unreadable. | Fix APK packaging or use the baseline llama artifact. |
| `VIFU-LLAMA-BACKEND-002` | Libraries were present, but no backend device registered. | Use the baseline artifact and report the device details. |
| `VIFU-LLAMA-MODEL-001` | The selected model cannot be inspected or read. | Ask the user to select or copy the GGUF again. |
| `VIFU-LLAMA-MODEL-002` | The selected model is empty. | Reject the file and request another GGUF. |
| `VIFU-LLAMA-MODEL-003` | Backends registered, but llama.cpp rejected the model. | Show the detailed error; optimized apps can offer the baseline build. |

Configuration mistakes cross FFI as `InvalidConfig`. Backend startup and model
loading failures cross FFI as `Runtime`, so callers can classify them without
parsing UI copy.

## Build the AARs

From this directory:

```bash
export ANDROID_NDK_HOME=/path/to/android-ndk
./gradlew :vifu-android-core:assembleRelease
./gradlew :vifu-android-llama:assembleRelease
./gradlew :vifu-android-llama-baseline:assembleRelease
./gradlew :vifu-android-whisper:assembleRelease
```

For a local application build that consumes the same Maven coordinates as a
release, publish only to Maven Local. Each publication includes the Kotlin
sources and an empty Javadoc artifact, so publishing does not depend on Dokka's
bytecode parser.

```bash
./gradlew -PVERSION_NAME=0.1.12 \
  :vifu-android-core:publishToMavenLocal \
  :vifu-android-llama:publishToMavenLocal \
  :vifu-android-whisper:publishToMavenLocal
```

The source generators can also run directly from the repository root:

```bash
scripts/build-android-package.sh --module core
scripts/build-android-package.sh --module llama --profile optimized
scripts/build-android-package.sh --module llama --profile baseline
scripts/build-android-package.sh --module whisper
```

Application inference runs locally. Gateway monitoring is optional.

## Starter

[`examples/android-starter`](../../examples/android-starter) is based on Arm's
official `examples/llama.android`
sample at commit `e5a10d0bbb990becf75167a691afd1359a30651e`. It preserves the
small choose-a-GGUF-and-chat interaction while replacing the inference adapter
with the split Vifu modules. Pass `-PvifuBackend=baseline` to exercise the
fallback llama artifact or `-PvifuWhisper=true` to verify both providers in one
APK. Vifu GitHub releases include signed optimized and baseline Starter APKs;
the source project is the advanced customization path.
