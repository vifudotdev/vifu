# Vifu Android Starter

This starter keeps the small interaction path from Arm's official
`examples/llama.android` sample: choose a GGUF file and chat with streaming
tokens. Vifu's split Android modules provide llama.cpp inference, the local
Agent Runtime, optional Gateway connectivity, cancellation, and tracing.

The default dependency is the ARM-optimized build. It discovers the best
compatible llama.cpp CPU backend at runtime and falls back to its ARMv8 backend
on less capable ARM64 phones. A separately built baseline AAR is available for
device-specific compatibility issues.

## Prerequisites

- An Android ARM64 phone with Android 13 or later
- Android SDK 36, Android NDK, JDK 17, and `adb`
- About 1.2 GB of free space while Android copies the downloaded model into the
  app's private storage

## Ten-minute local loop

### 1. Put a small chat model on the phone

```bash
curl -L --fail \
  https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/df5bf01389a39c743ab467d734bf501681e041c5/qwen2.5-0.5b-instruct-q4_k_m.gguf \
  -o qwen2.5-0.5b-instruct-q4_k_m.gguf
adb push qwen2.5-0.5b-instruct-q4_k_m.gguf /sdcard/Download/
```

The pinned file is 491,400,032 bytes and its SHA-256 is
`74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db`.

### 2. Start Vifu on the developer machine

Start the Vifu release binary on a LAN address reachable by the phone. The
Server, TUI, monitor stream, and Dashboard all run on the developer machine:

```bash
./vifu \
  -c server.address=https://<developer-lan-address>:6790 \
  -c server.guest_bootstrap.enabled=true \
  -c gateway.address=http://127.0.0.1:6790
```

Use the machine's LAN address, not `127.0.0.1`, for `server.address`. Keep this
terminal open.

### 3. Create one App and configure the Starter

Press `B`, create an App in the Dashboard, and copy its App ID. The Android app
needs that App ID; it does not need a Runtime ID.

```bash
./gradlew configureVifu \
  -PvifuAppId=vifu_app_<64-hex-characters> \
  -PvifuServerUrl=https://<developer-lan-address>:6790
./gradlew installDebug
```

`configureVifu` writes the ignored `vifu.properties` and copies Vifu's local
server certificate into the build configuration. If `server.address` is saved
under `[server]` in `~/.vifu/config.toml`, omit `-PvifuServerUrl`.

### 4. Run and inspect one turn

Open the app, choose the GGUF from Downloads, and send one message. The app
copies the model into private storage and reloads it on later launches. The
Gateway starts after the local agent is ready and then reconnects automatically
with its device credential. The Starter also keeps sending its App ID during
token resume, so a phone that has previously connected to another App still
syncs and uploads traces to the App selected in `vifu.properties`.

Success is visible in three places:

- the app shows `Vifu: connected`;
- the TUI lists `Android local llama`;
- the Dashboard shows the same invocation's model, stages, timings, and bounded
  errors.

Model execution stays inside the APK. Vifu stores monitoring data in the local
SQLite database on the developer machine. The running phone-to-Vifu loop uses
the LAN and does not depend on a hosted monitoring service. Prompt and response
content stay on the phone in the default configuration.

In the TUI, open the Android agent to inspect latency, TTFT, token rate, and the
Queue, Tokenize, Prefill, First token, Decode, and Validate stages. Press `B`
from that trace to open the same persistent record in the Dashboard. Vifu keeps
the Server, monitor stream, Dashboard, and SQLite trace store in the one local
binary.

### Connection checks

| App state | Check |
| --- | --- |
| `local` | Finish model selection; the Gateway starts with the local agent. |
| `connecting` or `reconnecting` | Confirm that the phone and developer machine share a LAN and the phone can reach the configured LAN address. |
| `authorization required` | Re-run `configureVifu` with the App ID copied from the current local Dashboard. |
| `degraded` or `failed` | Read the bounded status detail, then confirm the server URL and local certificate. |

The Vifu terminal must stay running. Closing only the Dashboard browser does not
stop the Server or TUI.

## Verified device result

The complete loop was exercised on August 12, 2026 with a Xiaomi
`2407FPN8ER`, Android 16, `arm64-v8a`, and SoC identifier `MT6989`. The Starter
used the optimized AAR, which loaded the packaged `android_armv9.0_1` GGML CPU
backend at runtime, and Qwen2.5 0.5B Q4_K_M.

The final fresh `ARM LIVE OK` turn produced this local trace after an automatic
device-token resume:

| Measurement | Result |
| --- | ---: |
| Embedded mobile runtime | 1,218 ms |
| Time to first token | 847 ms |
| Output rate | 17.1 tokens/s |
| Queue / Tokenize / Prefill | 2 / 6 / 784 ms |
| First token / Decode / Validate | 4 / 175 / 1 ms |

The trace reached the local SQLite store while the app remained connected, and
its durable phone outbox was empty after acknowledgement.

This is a reproducible single-device validation, not a claim of the same speed
or an optimized-versus-baseline speedup on every phone. Vifu keeps the exact
device, model, backend, and stage evidence visible so developers can run the
comparison that matters for their own target device.

## Choose the AAR

The optimized artifact is the default:

```kotlin
implementation("dev.vifu:vifu-android-llama:0.1.12")
```

If a specific phone cannot load it, rebuild with the baseline artifact:

```bash
./gradlew installDebug -PvifuBackend=baseline
```

This selects `dev.vifu:vifu-android-llama-baseline:0.1.12`. Both llama artifacts
expose the same Kotlin API, so application code does not change. Do not add both
llama artifacts to one app. Each pulls in `vifu-android-core` transitively.

To package the independent Whisper provider beside llama:

```bash
./gradlew installDebug -PvifuWhisper=true
```

This adds `dev.vifu:vifu-android-whisper:0.1.12`. The two providers share one
Core runtime when application code uses `VifuLlamaAgent.attach` and
`VifuWhisperAgent.attach`.

For development beside a Vifu checkout, add `-PvifuUseLocalCheckout=true`.
For a Maven Local test, add `-PvifuUseMavenLocal=true`.

## Minimal API

```kotlin
val agent = VifuLlamaAgent.open(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath, contextSize = 2_048u),
)

agent.send("Hello").collect { token -> render(token) }
```

`open` loads the optional llama module, registers its provider, and creates
the agent endpoint. Use the overload with `VifuConnectionConfig` to also start
the Gateway. Cancelling collection cancels inference.
Only successful turns are added to conversation history; call
`resetConversation()` to start over.

## Upstream

Based on Arm's `examples/llama.android` at commit
`e5a10d0bbb990becf75167a691afd1359a30651e`:
https://github.com/Arm/ai-chat

The upstream Apache 2.0 license and AUTHORS file are preserved.
