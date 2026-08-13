# Vifu Android Starter

The Android Starter is a prebuilt local chat app and a small reference project.
It keeps the interaction path from Arm's official `examples/llama.android`
sample: choose a GGUF model and chat with streaming tokens. Vifu adds the
embedded Agent Runtime, Gateway pairing, reconnect, cancellation, and tracing.

Start with the APK. Build the project only when you want to change the app or
embed Vifu in your own Android application.

## Install and run

Requirements:

- an ARM64 phone with Android 13 or later.
- about 1.2 GiB of free storage during model setup.

1. Download and extract the Vifu archive for your computer from the
   [latest release](https://github.com/vifudotdev/vifu/releases/latest).
2. Download the
   [optimized APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter.apk)
   and install it on the phone.
3. Open **Vifu Starter**. Choose **Download 469 MiB** to fetch the verified
   Qwen2.5 0.5B model, or import another GGUF.
4. Send a message. The app stores the model on the phone and restores it on the
   next launch.

The main APK uses Vifu's optimized ARM64 llama.cpp provider. If that provider
cannot start on a specific phone, install the
[baseline APK](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter-baseline.apk).
Android installs it beside the optimized APK. The launcher shows **Vifu Starter
Optimized** and **Vifu Starter Baseline**. Pair both applications with the same
Vifu project to compare their traces on one phone.

Android isolates the data for each application. Download the model in both
applications, or import the same GGUF into each application.

Release checksums are in
[vifu-android-starter-checksums.sha256](https://github.com/vifudotdev/vifu/releases/download/android-starter-v0.1.1/vifu-android-starter-checksums.sha256).

These APKs use a test signature for direct device evaluation. They are not
Google Play packages. If a future Demo uses another signature, uninstall both
old Demo applications before installation.

## Pair and inspect

Connect the phone and developer computer to the same local network. Start the
downloaded Vifu binary with a Server address that the phone can reach:

```bash
./vifu \
  -c server.address=https://<computer-lan-address>:6790 \
  -c server.guest_bootstrap.enabled=true \
  -c gateway.address=http://127.0.0.1:6790
```

Press `B`, open the project and its primary deployment in the Dashboard, then
choose **Pair gateway** and **Copy pairing code**. Tap the Vifu status row in
the app and paste the code. The app validates the one-time token and certificate
pin, stores its device credential in Android Keystore, and reconnects on later
launches.

Create a new pairing code for the second application. Select the same Vifu
project so both Agent traces appear together.

The Dashboard QR is also available for camera-based application links. The
copied native code is the reliable path for a local Server certificate because
it carries the complete trust anchor.

Run one chat turn. Success is visible in three places:

- the app shows `Vifu: connected`.
- the TUI lists `Android llama (ARM optimized)` or `Android llama (baseline)`.
- the Dashboard shows the invocation model, stages, timings, and bounded
  errors.

The model executes inside the Android app. Vifu stores monitoring data in the
SQLite database on the developer computer. The default trace records timing,
stage status, and bounded errors while prompt and response content remain in
the app.

## Build from source

The source project is the advanced path. It requires Android SDK 36, an Android
NDK, JDK 17, and `adb`.

Build from the Vifu checkout:

```bash
cd examples/android-starter
./gradlew installDebug -PvifuUseLocalCheckout=true
```

Use the baseline provider for a compatibility build:

```bash
./gradlew installDebug -PvifuUseLocalCheckout=true -PvifuBackend=baseline
```

Add the independent Whisper provider when the host app needs transcription:

```bash
./gradlew installDebug -PvifuUseLocalCheckout=true -PvifuWhisper=true
```

The optional `configureVifu` task remains available for automated builds that
bind a specific App ID and certificate at build time. Add
`-PvifuUseBuildTimePairing=true` to that build. Normal source builds and the
Demo APKs use runtime pairing and ignore `vifu.properties`.

## Minimal embedding API

```kotlin
val agent = VifuLlamaAgent.open(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath, contextSize = 2_048u),
)

agent.send("Hello").collect { token -> render(token) }
```

`open` loads the selected llama module, registers its provider, and creates the
agent endpoint. Use `VifuGatewayPairingCode` and the connection overload to add
local monitoring:

```kotlin
val connection = VifuGatewayPairingCode(pairingCode).connectionConfig()
val agent = VifuLlamaAgent.open(applicationContext, connection, model)
```

Cancelling collection cancels inference. Only successful turns are added to
conversation history. Call `resetConversation()` to start over.

## Verified device result

The full optimized path was exercised on August 12, 2026 with a Xiaomi
`2407FPN8ER`, Android 16, `arm64-v8a`, and SoC identifier `MT6989`. The packaged
`android_armv9.0_1` backend ran Qwen2.5 0.5B Q4_K_M and uploaded the completed
trace to the local Vifu SQLite store.

| Measurement | Result |
| --- | ---: |
| Embedded mobile runtime | 1,218 ms |
| Time to first token | 847 ms |
| Output rate | 17.1 tokens/s |
| Queue / Tokenize / Prefill | 2 / 6 / 784 ms |
| First token / Decode / Validate | 4 / 175 / 1 ms |

This is one reproducible device result. Vifu exposes the device, model,
backend, and stage evidence so developers can measure their own target phones.

## Upstream

Based on Arm's `examples/llama.android` at commit
`e5a10d0bbb990becf75167a691afd1359a30651e`:
https://github.com/Arm/ai-chat

The upstream Apache 2.0 license and AUTHORS file are preserved.
