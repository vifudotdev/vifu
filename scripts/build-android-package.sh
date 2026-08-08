#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/vifu-android}"
DIST_DIR="${VIFU_ANDROID_DIST_DIR:-$REPO_ROOT/target/vifu-android-dist}"
ANDROID_TARGET="aarch64-linux-android"
ANDROID_ABI="arm64-v8a"
ANDROID_API_LEVEL="${VIFU_ANDROID_API_LEVEL:-24}"
FFI_CRATE="vifu-mobile-ffi"
FFI_FEATURES="${VIFU_ANDROID_FFI_FEATURES:-}"
BINDINGS_ONLY=false

usage() {
    cat <<'EOF'
Usage: scripts/build-android-package.sh [--bindings-only]

Builds the Vifu Android arm64 native library and matching UniFFI Kotlin source
into a Gradle source-set layout under target/vifu-android-dist.

Environment:
  ANDROID_NDK_HOME          Android NDK root (ANDROID_NDK_ROOT also works)
  VIFU_ANDROID_DIST_DIR     Output directory
  VIFU_ANDROID_API_LEVEL    Native API level (default: 24)
  VIFU_ANDROID_FFI_FEATURES Optional comma-separated Cargo features

The default Android artifact excludes optional on-device model providers. Set
VIFU_ANDROID_FFI_FEATURES=local-llama,local-whisper when those providers are
required by the host application.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bindings-only)
            BINDINGS_ONLY=true
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
    shift
done

feature_args=(--no-default-features)
if [[ -n "$FFI_FEATURES" ]]; then
    feature_args+=(--features "$FFI_FEATURES")
fi

export CARGO_TARGET_DIR="$TARGET_DIR"
export GGML_CCACHE="${GGML_CCACHE:-OFF}"

cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p "$FFI_CRATE" \
    "${feature_args[@]}"
cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p "$FFI_CRATE" \
    --bin uniffi-bindgen \
    "${feature_args[@]}"

case "$(uname -s)" in
    Darwin) host_library="$TARGET_DIR/debug/libvifu_mobile_ffi.dylib" ;;
    Linux) host_library="$TARGET_DIR/debug/libvifu_mobile_ffi.so" ;;
    *)
        echo "Unsupported build host: $(uname -s)" >&2
        exit 1
        ;;
esac

temp_root="${VIFU_ANDROID_TEMP_DIR:-${RUNNER_TEMP:-/private/tmp}}"
mkdir -p "$temp_root"
work_dir="$(mktemp -d "$temp_root/vifu-android-package.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

bindings_dir="$work_dir/bindings"
mkdir -p "$bindings_dir"
env -u CARGO_TARGET_DIR "$TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$host_library" \
    --language kotlin \
    --out-dir "$bindings_dir" \
    --config "$REPO_ROOT/crates/vifu-mobile-ffi/uniffi.toml" \
    --no-format

generated_kotlin="$bindings_dir/dev/vifu/runtime/vifu_mobile_ffi.kt"
if [[ ! -s "$generated_kotlin" ]]; then
    echo "UniFFI did not generate the expected Kotlin source." >&2
    exit 1
fi

kotlin_output="$DIST_DIR/src/main/kotlin/dev/vifu/runtime"
mkdir -p "$kotlin_output"
cp "$generated_kotlin" "$kotlin_output/vifu_mobile_ffi.kt"

if [[ "$BINDINGS_ONLY" == "true" ]]; then
    printf 'Generated Vifu Android Kotlin bindings at %s\n' "$DIST_DIR"
    exit 0
fi

installed_targets="$(rustup target list --installed)"
if ! grep -qx "$ANDROID_TARGET" <<<"$installed_targets"; then
    echo "Missing Rust target: $ANDROID_TARGET" >&2
    echo "Install it explicitly with: rustup target add $ANDROID_TARGET" >&2
    exit 1
fi

ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-${ANDROID_NDK_LATEST_HOME:-}}}"
if [[ -z "$ndk_root" ]]; then
    echo "ANDROID_NDK_HOME is required for the Android native build." >&2
    exit 1
fi

case "$(uname -s)" in
    Darwin) ndk_host="darwin-x86_64" ;;
    Linux) ndk_host="linux-x86_64" ;;
esac
ndk_bin="$ndk_root/toolchains/llvm/prebuilt/$ndk_host/bin"
ndk_sysroot="$ndk_root/toolchains/llvm/prebuilt/$ndk_host/sysroot"
clang="$ndk_bin/aarch64-linux-android${ANDROID_API_LEVEL}-clang"
clangxx="$ndk_bin/aarch64-linux-android${ANDROID_API_LEVEL}-clang++"
if [[ ! -x "$clang" ]] || [[ ! -x "$clangxx" ]] || [[ ! -x "$ndk_bin/llvm-ar" ]]; then
    echo "Android NDK toolchain is incomplete under: $ndk_bin" >&2
    exit 1
fi

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$clang"
export CC_aarch64_linux_android="$clang"
export CXX_aarch64_linux_android="$clangxx"
export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
export ANDROID_NDK="$ndk_root"
export ANDROID_NDK_ROOT="$ndk_root"
export NDK_ROOT="$ndk_root"
export ANDROID_API_LEVEL="$ANDROID_API_LEVEL"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ndk_sysroot"

# whisper-rs-sys 0.15 evaluates target_os in its host build script. A macOS
# host therefore requests ggml-blas even when the Android CMake build correctly
# disables BLAS. The archive is intentionally empty: no Android object refers
# to BLAS symbols when GGML_BLAS=OFF. Keep this compatibility shim scoped to the
# Android target until the upstream build script uses CARGO_CFG_TARGET_OS.
android_compat_libs="$TARGET_DIR/android-host-compat"
mkdir -p "$android_compat_libs"
"$ndk_bin/llvm-ar" crs "$android_compat_libs/libggml-blas.a"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} -Lnative=$android_compat_libs"

cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    --release \
    -p "$FFI_CRATE" \
    --target "$ANDROID_TARGET" \
    "${feature_args[@]}"

native_library="$TARGET_DIR/$ANDROID_TARGET/release/libvifu_mobile_ffi.so"
if [[ ! -s "$native_library" ]]; then
    echo "Android native library was not produced: $native_library" >&2
    exit 1
fi

jni_output="$DIST_DIR/src/main/jniLibs/$ANDROID_ABI"
mkdir -p "$jni_output"
cp "$native_library" "$jni_output/libvifu_mobile_ffi.so"

printf 'Built Vifu Android package sources at %s\n' "$DIST_DIR"
