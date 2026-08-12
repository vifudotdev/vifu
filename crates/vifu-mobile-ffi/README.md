# Vifu Mobile FFI

This crate is the native Core adapter for embedding Vifu in iOS and Android
applications. It exposes the self-contained Agent Runtime and Gateway through
UniFFI.

`VifuEmbeddedRuntime` provides:

- dynamic native and streaming provider callbacks;
- provider, agent, and named endpoint registration and removal;
- non-blocking start, poll, take, and cancel operations;
- JSON and binary invocation data;
- project snapshot export and restore.

The embedded Runtime executes in the application process. Provider callbacks
run outside the Runtime executor, so a blocking native call cannot stall
invocation timeouts or unrelated Runtime work. Cancellation returns control to
the Runtime immediately; providers also receive the cancellation signal and
streaming event sink.

`VifuEmbeddedGateway.start` publishes performance telemetry but keeps root
invocation input and output on the device. Hosts that provide an explicit
content-sharing consent control can instead call `startWithMonitorIo` /
`start_with_monitor_io` with `captureMonitorIo = true` for a private debugging
session.

## Apple package

Apple application developers can add `https://github.com/vifudotdev/vifu` as a
Swift Package and select the `Vifu` product. The release package contains the
generated Swift source and a checksum-verified XCFramework built with the
default mobile provider features, including in-process llama and Local Whisper.

Build the Apple distribution artifact from the repository root:

```bash
rustup target add \
  aarch64-apple-ios \
  aarch64-apple-ios-sim \
  x86_64-apple-ios \
  x86_64-apple-darwin
scripts/build-apple-package.sh
```

Refresh the tracked Swift wrapper after changing the UniFFI surface:

```bash
scripts/build-apple-package.sh --update-bindings
```

## Android modules

Android packages Core and the local providers as separate AARs and native
entry points:

```kotlin
implementation("dev.vifu:vifu-android-core:0.1.12")
implementation("dev.vifu:vifu-android-llama:0.1.12")
implementation("dev.vifu:vifu-android-whisper:0.1.12")
```

Provider artifacts depend on Core, so applications normally declare only the
providers they use. The baseline llama coordinate is
`dev.vifu:vifu-android-llama-baseline:0.1.12`.

Build one Android `arm64-v8a` source set at a time from the repository root:

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk \
  scripts/build-android-package.sh --module core
ANDROID_NDK_HOME=/path/to/android-ndk \
  scripts/build-android-package.sh --module llama --profile optimized
ANDROID_NDK_HOME=/path/to/android-ndk \
  scripts/build-android-package.sh --module whisper
```

Set `VIFU_ANDROID_DIST_DIR` to choose the generated Gradle source-set output.
Kotlin bindings are written under `src/main/kotlin/` and native libraries under
`src/main/jniLibs/arm64-v8a/`.

Generate only a module's Kotlin bindings when validating the interface on a
machine without the Rust Android target:

```bash
scripts/build-android-package.sh --module core --bindings-only
scripts/build-android-package.sh --module llama --bindings-only
scripts/build-android-package.sh --module whisper --bindings-only
```

See [`integrations/android`](../../integrations/android/README.md) for the
high-level lifecycle API and artifact builds. See the
[`Mobile Starter`](../../examples/mobile-starter/README.md) for the prebuilt
Android app and the corresponding iOS path.
