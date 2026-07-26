# Vifu Mobile FFI

This crate is the native mobile adapter for Vifu runtime bindings. It links the
Vifu gateway runtime into one library and exposes a small UniFFI surface for
iOS and Android clients.

Current component:

- `vifu-gateway`

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
