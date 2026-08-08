# Vifu Mobile FFI

This crate is the native mobile adapter for embedding Vifu in iOS and Android
applications. It exposes the self-contained Runtime and the existing Gateway
configuration and probe utilities through UniFFI.

`VifuEmbeddedRuntime` provides:

- dynamic native `VifuAgentProvider` callbacks;
- provider, agent, and named endpoint registration;
- non-blocking start, poll, take, and cancel operations for application loops;
- JSON and binary invocation data;
- project snapshot export and restore.

The embedded Runtime executes in the application process. Native provider
callbacks connect it to the host's local or remote agent implementations.
Provider callbacks run outside the Runtime executor so a blocking native call
cannot stall invocation timeouts or unrelated Runtime work. Cancellation
returns control to the Runtime immediately; native providers should still
cooperate with their platform cancellation APIs when they can.
Agent capabilities are declared when the host registers an agent, so capability
checks do not require a synchronous native callback.

`VifuEmbeddedGateway.start` publishes performance telemetry but keeps root
invocation input and output on the device. Hosts that provide an explicit
content-sharing consent control can instead call `startWithMonitorIo` /
`start_with_monitor_io` with `captureMonitorIo = true` for a private debugging
session.

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

Build the Android `arm64-v8a` package sources from the repository root:

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk scripts/build-android-package.sh
```

The generated Gradle source-set layout is under
`target/vifu-android-dist/src/main/`: Kotlin bindings are in `kotlin/` and the
native library is in `jniLibs/arm64-v8a/`. The default mobile artifact keeps
the Runtime and Gateway but omits optional on-device model providers. Set
`VIFU_ANDROID_FFI_FEATURES=local-llama,local-whisper` when the Android host
intentionally ships those providers.

Generate only the Kotlin bindings when validating the interface on a machine
without the Rust Android target:

```bash
scripts/build-android-package.sh --bindings-only
```

Generate bindings with the crate-local UniFFI bindgen binary and
`crates/vifu-mobile-ffi/uniffi.toml`:

```bash
cargo build -p vifu-mobile-ffi
cargo build -p vifu-mobile-ffi --bin uniffi-bindgen
target/debug/uniffi-bindgen generate \
  --library target/debug/libvifu_mobile_ffi.dylib \
  --language swift \
  --language kotlin \
  --out-dir target/vifu-mobile-bindings \
  --config crates/vifu-mobile-ffi/uniffi.toml
```
