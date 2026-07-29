# Vifu Mobile FFI

This crate is the native mobile adapter for embedding Vifu in iOS and Android
applications. It exposes the self-contained Runtime and the existing Gateway
configuration and probe utilities through UniFFI.

`VifuEmbeddedRuntime` provides:

- dynamic native `VifuAgentProvider` callbacks;
- provider, agent, and named endpoint registration;
- non-blocking start, poll, and cancel operations for application loops;
- JSON and binary invocation data;
- project snapshot export and restore.

The embedded Runtime executes in the application process. Native provider
callbacks connect it to the host's local or remote agent implementations.

Build examples:

```bash
cd vifu
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo build -p vifu-mobile-ffi --target aarch64-apple-ios --release
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/android-ndk
cargo build -p vifu-mobile-ffi --target aarch64-linux-android --release
```

Generate bindings with the crate-local UniFFI bindgen binary and
`crates/vifu-mobile-ffi/uniffi.toml`.
