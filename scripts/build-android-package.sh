#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/vifu-android}"
HOST_TARGET_DIR="${VIFU_ANDROID_HOST_TARGET_DIR:-$REPO_ROOT/target}"
DIST_DIR_OVERRIDE="${VIFU_ANDROID_DIST_DIR:-}"
ANDROID_TARGET="aarch64-linux-android"
ANDROID_ABI="arm64-v8a"
ANDROID_API_LEVEL="${VIFU_ANDROID_API_LEVEL:-24}"
FFI_MODULE="core"
BUILD_PROFILE="optimized"
PROFILE_SET=false
BINDINGS_ONLY=false

usage() {
    cat <<'EOF'
Usage: scripts/build-android-package.sh --module core|llama|whisper [--profile optimized|baseline] [--bindings-only]

Builds one Vifu Android arm64 native module and its UniFFI Kotlin source into a
module-specific Gradle source-set layout under target/.

Environment:
  ANDROID_NDK_HOME          Android NDK root (ANDROID_NDK_ROOT also works)
  VIFU_ANDROID_DIST_DIR     Output directory
  VIFU_ANDROID_API_LEVEL    Native API level (default: 24)
The optimized profile packages llama.cpp's runtime-selected ARM CPU variants.
The baseline profile packages a static ARMv8 llama.cpp build. Profiles apply
only to the llama module.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bindings-only)
            BINDINGS_ONLY=true
            ;;
        --profile)
            shift
            BUILD_PROFILE="${1:-}"
            PROFILE_SET=true
            ;;
        --module)
            shift
            FFI_MODULE="${1:-}"
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

case "$BUILD_PROFILE" in
    optimized|baseline) ;;
    *)
        echo "Unknown Android profile: $BUILD_PROFILE" >&2
        usage >&2
        exit 2
        ;;
esac

host_feature_args=(--no-default-features)
native_feature_args=(--no-default-features)
case "$FFI_MODULE" in
    core)
        ffi_crate="vifu-mobile-ffi"
        library_name="vifu_mobile_ffi"
        binding_config="$REPO_ROOT/crates/vifu-mobile-ffi/uniffi.toml"
        generated_kotlin_relative="dev/vifu/runtime/vifu_mobile_ffi.kt"
        ;;
    llama)
        ffi_crate="vifu-llama-ffi"
        library_name="vifu_llama_ffi"
        binding_config="$REPO_ROOT/crates/vifu-llama-ffi/uniffi.toml"
        generated_kotlin_relative="dev/vifu/llama/vifu_llama_ffi.kt"
        if [[ "$BUILD_PROFILE" == "optimized" ]]; then
            native_feature_args+=(--features dynamic-backends)
        fi
        ;;
    whisper)
        ffi_crate="vifu-whisper-ffi"
        library_name="vifu_whisper_ffi"
        binding_config="$REPO_ROOT/crates/vifu-whisper-ffi/uniffi.toml"
        generated_kotlin_relative="dev/vifu/whisper/vifu_whisper_ffi.kt"
        ;;
    *)
        echo "Unknown Android module: $FFI_MODULE" >&2
        usage >&2
        exit 2
        ;;
esac

if [[ "$FFI_MODULE" != "llama" && "$PROFILE_SET" == "true" ]]; then
    echo "--profile applies only to the llama module." >&2
    usage >&2
    exit 2
fi

if [[ -n "$DIST_DIR_OVERRIDE" ]]; then
    DIST_DIR="$DIST_DIR_OVERRIDE"
elif [[ "$FFI_MODULE" == "llama" ]]; then
    DIST_DIR="$REPO_ROOT/target/vifu-android-${FFI_MODULE}-${BUILD_PROFILE}-dist"
else
    DIST_DIR="$REPO_ROOT/target/vifu-android-${FFI_MODULE}-dist"
fi

export GGML_CCACHE="${GGML_CCACHE:-OFF}"

env CARGO_TARGET_DIR="$HOST_TARGET_DIR" cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p "$ffi_crate" \
    "${host_feature_args[@]}"
env CARGO_TARGET_DIR="$HOST_TARGET_DIR" cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p vifu-mobile-ffi \
    --bin uniffi-bindgen \
    --no-default-features

case "$(uname -s)" in
    Darwin) host_library="$HOST_TARGET_DIR/debug/lib${library_name}.dylib" ;;
    Linux) host_library="$HOST_TARGET_DIR/debug/lib${library_name}.so" ;;
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
"$HOST_TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$host_library" \
    --language kotlin \
    --out-dir "$bindings_dir" \
    --config "$binding_config" \
    --no-format

generated_kotlin="$bindings_dir/$generated_kotlin_relative"
if [[ ! -s "$generated_kotlin" ]]; then
    echo "UniFFI did not generate the expected Kotlin source." >&2
    exit 1
fi

kotlin_output="$DIST_DIR/src/main/kotlin/$(dirname "$generated_kotlin_relative")"
mkdir -p "$kotlin_output"
cp "$generated_kotlin" "$kotlin_output/$(basename "$generated_kotlin_relative")"

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
export CARGO_TARGET_DIR="$TARGET_DIR"
export CC_aarch64_linux_android="$clang"
export CXX_aarch64_linux_android="$clangxx"
export AR_aarch64_linux_android="$ndk_bin/llvm-ar"
export ANDROID_NDK="$ndk_root"
export ANDROID_NDK_ROOT="$ndk_root"
export NDK_ROOT="$ndk_root"
export ANDROID_API_LEVEL="$ANDROID_API_LEVEL"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ndk_sysroot"

# Keep every CMake-backed native dependency on the same Android toolchain. Use
# API-specific NDK compilers and empty base flags so a macOS host cannot leak
# Darwin -arch or -isysroot flags into the cross build.
export CMAKE_TOOLCHAIN_FILE="$REPO_ROOT/scripts/cmake/android.toolchain.cmake"
export CMAKE_SYSTEM_NAME=Android
export CMAKE_SYSTEM_VERSION="$ANDROID_API_LEVEL"
export CMAKE_ANDROID_ARCH_ABI="$ANDROID_ABI"
export CMAKE_ANDROID_NDK="$ndk_root"
export CMAKE_C_COMPILER="$clang"
export CMAKE_CXX_COMPILER="$clangxx"
export CMAKE_ASM_COMPILER="$clang"
export CMAKE_C_FLAGS=""
export CMAKE_CXX_FLAGS=""
export CMAKE_ASM_FLAGS=""
export ANDROID_ABI="$ANDROID_ABI"
export ANDROID_PLATFORM="android-$ANDROID_API_LEVEL"
export GGML_ACCELERATE=OFF
export GGML_BLAS=OFF
# Repacking duplicates most model weights during initialization. That peak is
# acceptable in a standalone benchmark but can make model loading fail inside
# a Godot + React Native Android process.
export GGML_CPU_REPACK=OFF
export BUILD_TESTING=OFF
export WHISPER_BUILD_EXAMPLES=OFF
export WHISPER_BUILD_TESTS=OFF
# Dynamic llama.cpp ARM variants remain enabled by the optimized Cargo
# feature. Keep this global CMake flag off because whisper.cpp otherwise tries
# to install a KleidiAI archive that its static Android build does not emit.
export GGML_CPU_KLEIDIAI=OFF

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
    -p "$ffi_crate" \
    --target "$ANDROID_TARGET" \
    "${native_feature_args[@]}"

native_library="$TARGET_DIR/$ANDROID_TARGET/release/lib${library_name}.so"
if [[ ! -s "$native_library" ]]; then
    echo "Android native library was not produced: $native_library" >&2
    exit 1
fi
readelf="$ndk_bin/llvm-readelf"
if [[ "$FFI_MODULE" == "whisper" ]] &&
    "$readelf" -Ws "$native_library" | grep -Eq 'UND[[:space:]]+whisper_'; then
    echo "Android native library contains unresolved Whisper symbols." >&2
    exit 1
fi
if [[ "$FFI_MODULE" == "llama" && "$BUILD_PROFILE" == "optimized" ]] &&
    "$readelf" -Ws "$native_library" |
        grep -Eq 'LOCAL[[:space:]]+DEFAULT[[:space:]]+[0-9]+[[:space:]]+ggml_backend_load_all_from_path$'; then
    echo "Optimized Android FFI contains a private GGML backend registry; check for statically linked GGML providers." >&2
    exit 1
fi

jni_output="$DIST_DIR/src/main/jniLibs/$ANDROID_ABI"
# This directory is generated as one package unit. Remove the previous ABI
# payload so rebuilding after a native code change cannot compare new backend
# libraries with stale copies from an earlier package.
case "$jni_output" in
    "$DIST_DIR"/src/main/jniLibs/*) rm -rf -- "$jni_output" ;;
    *)
        echo "Refusing to clean unexpected JNI output path: $jni_output" >&2
        exit 1
        ;;
esac
mkdir -p "$jni_output"
cp "$native_library" "$jni_output/lib${library_name}.so"
cxx_runtime="$ndk_sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
if [[ ! -s "$cxx_runtime" ]]; then
    echo "Android C++ runtime was not found: $cxx_runtime" >&2
    exit 1
fi
if [[ "$FFI_MODULE" == "core" ]]; then
    cp "$cxx_runtime" "$jni_output/libc++_shared.so"
fi

if [[ "$FFI_MODULE" == "llama" && "$BUILD_PROFILE" == "optimized" ]]; then
    backend_root="$TARGET_DIR/$ANDROID_TARGET/release/build"
    while IFS= read -r shared_library; do
        destination="$jni_output/$(basename "$shared_library")"
        if [[ -f "$destination" ]] && ! cmp -s "$shared_library" "$destination"; then
            echo "Conflicting optimized libraries named $(basename "$shared_library")" >&2
            exit 1
        fi
        cp "$shared_library" "$destination"
    done < <(find "$backend_root" -type f -path '*/out/*' -name '*.so' -print | sort -u)
    if ! find "$jni_output" -maxdepth 1 -type f -name 'libggml-cpu-*.so' -print -quit | grep -q .; then
        echo "Optimized build did not produce dynamic ARM CPU backend libraries." >&2
        exit 1
    fi
fi

while IFS= read -r packaged_library; do
    while IFS= read -r dependency; do
        case "$dependency" in
            libc.so|libdl.so|liblog.so|libm.so|libandroid.so|libc++_shared.so) ;;
            *)
                if [[ ! -s "$jni_output/$dependency" ]]; then
                    echo "Missing packaged dependency $dependency required by $(basename "$packaged_library")" >&2
                    exit 1
                fi
                ;;
        esac
    done < <("$readelf" -d "$packaged_library" | sed -n 's/.*Shared library: \[\([^]]*\)\].*/\1/p')
done < <(find "$jni_output" -maxdepth 1 -type f -name '*.so' -print | sort)

if [[ "$FFI_MODULE" == "llama" ]]; then
    printf 'Built Vifu Android %s module (%s) package sources at %s\n' \
        "$FFI_MODULE" "$BUILD_PROFILE" "$DIST_DIR"
else
    printf 'Built Vifu Android %s module package sources at %s\n' \
        "$FFI_MODULE" "$DIST_DIR"
fi
