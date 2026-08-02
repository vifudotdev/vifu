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

Android build example:

```bash
cd vifu
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/android-ndk
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64/bin/aarch64-linux-android24-clang"
cargo build -p vifu-mobile-ffi --target aarch64-linux-android --release
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
