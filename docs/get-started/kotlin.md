# Build A Kotlin Android Agent With Vifu

Vifu Android separates the Core Runtime from the llama and Whisper Providers.
An application includes only the model modules it uses.

## 1. Add A Provider

For local chat:

```kotlin
dependencies {
    implementation("dev.vifu:vifu-android-llama:0.1.13")
}
```

For chat and transcription, add `dev.vifu:vifu-android-whisper:0.1.13` too.
Gradle resolves the shared Core Runtime once.

## 2. Open The Agent

Copy a compatible GGUF model into app-accessible storage, then open the Agent
on an IO coroutine:

```kotlin
val agent = VifuLlamaAgent.open(
    context = applicationContext,
    model = VifuLlamaConfig(modelPath = modelFile.absolutePath),
)
```

## 3. Stream A Reply

```kotlin
agent.send("Hello").collect { token ->
    render(token)
}
```

Cancelling collection cancels inference. Call `agent.close()` to unregister
the Provider and release its model resources.

## 4. Share One Runtime Across Models

```kotlin
val runtime = VifuAndroidRuntime.open(applicationContext)
val llama = VifuLlamaAgent.attach(runtime, VifuLlamaConfig(chatModel.absolutePath))
val whisper = VifuWhisperAgent.attach(runtime, VifuWhisperConfig(whisperModel.absolutePath))

val prompt = whisper.transcribe(wavBytes)
llama.send(prompt).collect { token -> render(token) }
```

Each Provider can load and unload independently. One Gateway advertises the
current Agent roster for the shared Runtime.

## 5. Pair With Vifu

Pass a scanned code only for the first enrollment:

```kotlin
val agent = VifuLlamaAgent.connect(
    context = applicationContext,
    model = VifuLlamaConfig(modelFile.absolutePath),
    pairingCode = pairingCode,
)
```

On later starts, omit `pairingCode`. Android Keystore protects the machine key
and device token. Trace content stays private unless the app enables
`captureTraceContent` after consent.

Use the [Android guide](../../integrations/android/README.md) for diagnostics,
baseline/optimized builds, and direct Core configuration. Use the
[Android Starter](../../examples/android-starter/) as the runnable source
project.
