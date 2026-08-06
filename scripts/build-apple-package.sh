#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/vifu-apple}"
DIST_DIR="${VIFU_APPLE_DIST_DIR:-$REPO_ROOT/target/vifu-apple-dist}"
LOCAL_XCFRAMEWORK="$REPO_ROOT/Frameworks/VifuMobileFFI.xcframework"
SWIFT_SOURCE="$REPO_ROOT/apple/Sources/Vifu/Vifu.swift"
FFI_CRATE="vifu-mobile-ffi"
FFI_LIBRARY="libvifu_mobile_ffi.a"
FFI_FEATURES="${VIFU_APPLE_FFI_FEATURES:-local-llama,local-whisper}"
UPDATE_BINDINGS=false
MACOS_ONLY=false
CI_ONLY=false

usage() {
    cat <<'EOF'
Usage: scripts/build-apple-package.sh [--update-bindings] [--macos-only] [--ci]

Builds VifuMobileFFI.xcframework for iOS, iOS Simulator, and macOS.
The optional flag refreshes the tracked UniFFI Swift wrapper.
Use --macos-only for a smaller local macOS development artifact.
Use --ci for an arm64 macOS and iOS Simulator verification artifact.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --update-bindings)
            UPDATE_BINDINGS=true
            ;;
        --macos-only)
            MACOS_ONLY=true
            ;;
        --ci)
            CI_ONLY=true
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

if [[ "$MACOS_ONLY" == "true" && "$CI_ONLY" == "true" ]]; then
    echo "--macos-only and --ci cannot be used together." >&2
    exit 2
fi

if [[ "$MACOS_ONLY" == "true" ]]; then
    required_targets=(aarch64-apple-darwin)
elif [[ "$CI_ONLY" == "true" ]]; then
    required_targets=(aarch64-apple-ios-sim aarch64-apple-darwin)
else
    required_targets=(
        aarch64-apple-ios
        aarch64-apple-ios-sim
        x86_64-apple-ios
        aarch64-apple-darwin
        x86_64-apple-darwin
    )
fi

if [[ "$UPDATE_BINDINGS" != "true" ]]; then
    installed_targets="$(rustup target list --installed)"
    for target in "${required_targets[@]}"; do
        if ! grep -qx "$target" <<<"$installed_targets"; then
            echo "Missing Rust target: $target" >&2
            echo "Install the Apple targets with:" >&2
            echo "  rustup target add ${required_targets[*]}" >&2
            exit 1
        fi
    done
fi

temp_root="${VIFU_APPLE_TEMP_DIR:-${RUNNER_TEMP:-/private/tmp}}"
mkdir -p "$temp_root"
work_dir="$(mktemp -d "$temp_root/vifu-apple-package.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

export CARGO_TARGET_DIR="$TARGET_DIR"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-17.0}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export GGML_CCACHE="${GGML_CCACHE:-OFF}"

if [[ "$UPDATE_BINDINGS" != "true" ]]; then
    for target in "${required_targets[@]}"; do
        cargo build \
            --manifest-path "$REPO_ROOT/Cargo.toml" \
            --locked \
            --release \
            -p "$FFI_CRATE" \
            --features "$FFI_FEATURES" \
            --target "$target"
    done
fi

bindings_dir="$work_dir/bindings"
mkdir -p "$bindings_dir"
cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p "$FFI_CRATE" \
    --features "$FFI_FEATURES"
cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    -p "$FFI_CRATE" \
    --features "$FFI_FEATURES" \
    --bin uniffi-bindgen
env -u CARGO_TARGET_DIR "$TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$TARGET_DIR/debug/libvifu_mobile_ffi.dylib" \
    --language swift \
    --out-dir "$bindings_dir" \
    --config "$REPO_ROOT/crates/vifu-mobile-ffi/uniffi.toml"

generated_swift="$bindings_dir/vifu_mobile_ffi.swift"
generated_header="$bindings_dir/vifu_mobile_ffiFFI.h"
generated_modulemap="$bindings_dir/vifu_mobile_ffiFFI.modulemap"
perl -pi -e 's/[ \t]+$//' "$generated_swift"

if [[ "$UPDATE_BINDINGS" == "true" ]]; then
    mkdir -p "$(dirname "$SWIFT_SOURCE")"
    cp "$generated_swift" "$SWIFT_SOURCE"
    printf 'Updated Swift bindings: %s\n' "$SWIFT_SOURCE"
    exit 0
elif [[ ! -f "$SWIFT_SOURCE" ]] || ! cmp -s "$generated_swift" "$SWIFT_SOURCE"; then
    echo "The tracked Swift wrapper is stale." >&2
    echo "Run scripts/build-apple-package.sh --update-bindings and commit the result." >&2
    exit 1
fi

headers_dir="$work_dir/headers"
mkdir -p "$headers_dir"
cp "$generated_header" "$headers_dir/vifu_mobile_ffiFFI.h"
cp "$generated_modulemap" "$headers_dir/module.modulemap"

macos_dir="$work_dir/macos"
mkdir -p "$macos_dir"

if [[ "$MACOS_ONLY" == "true" ]]; then
    cp \
        "$TARGET_DIR/aarch64-apple-darwin/release/$FFI_LIBRARY" \
        "$macos_dir/libVifuMobileFFI.a"
elif [[ "$CI_ONLY" == "true" ]]; then
    simulator_dir="$work_dir/ios-simulator"
    mkdir -p "$simulator_dir"
    cp \
        "$TARGET_DIR/aarch64-apple-ios-sim/release/$FFI_LIBRARY" \
        "$simulator_dir/libVifuMobileFFI.a"
    cp \
        "$TARGET_DIR/aarch64-apple-darwin/release/$FFI_LIBRARY" \
        "$macos_dir/libVifuMobileFFI.a"
else
    device_dir="$work_dir/ios-device"
    simulator_dir="$work_dir/ios-simulator"
    mkdir -p "$device_dir" "$simulator_dir"
    cp \
        "$TARGET_DIR/aarch64-apple-ios/release/$FFI_LIBRARY" \
        "$device_dir/libVifuMobileFFI.a"
    lipo -create \
        "$TARGET_DIR/aarch64-apple-ios-sim/release/$FFI_LIBRARY" \
        "$TARGET_DIR/x86_64-apple-ios/release/$FFI_LIBRARY" \
        -output "$simulator_dir/libVifuMobileFFI.a"
    lipo -create \
        "$TARGET_DIR/aarch64-apple-darwin/release/$FFI_LIBRARY" \
        "$TARGET_DIR/x86_64-apple-darwin/release/$FFI_LIBRARY" \
        -output "$macos_dir/libVifuMobileFFI.a"
fi

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
xcframework="$DIST_DIR/VifuMobileFFI.xcframework"
if [[ "$MACOS_ONLY" == "true" ]]; then
    xcodebuild -create-xcframework \
        -library "$macos_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -output "$xcframework"
elif [[ "$CI_ONLY" == "true" ]]; then
    xcodebuild -create-xcframework \
        -library "$simulator_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -library "$macos_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -output "$xcframework"
else
    xcodebuild -create-xcframework \
        -library "$device_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -library "$simulator_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -library "$macos_dir/libVifuMobileFFI.a" -headers "$headers_dir" \
        -output "$xcframework"
fi

# xcodebuild may emit AvailableLibraries in a different order between runs.
# Normalize the plist so SwiftPM receives a reproducible archive checksum.
if [[ "$MACOS_ONLY" != "true" && "$CI_ONLY" != "true" ]]; then
cat > "$xcframework/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>AvailableLibraries</key>
    <array>
        <dict>
            <key>BinaryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>HeadersPath</key>
            <string>Headers</string>
            <key>LibraryIdentifier</key>
            <string>ios-arm64</string>
            <key>LibraryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>SupportedArchitectures</key>
            <array>
                <string>arm64</string>
            </array>
            <key>SupportedPlatform</key>
            <string>ios</string>
        </dict>
        <dict>
            <key>BinaryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>HeadersPath</key>
            <string>Headers</string>
            <key>LibraryIdentifier</key>
            <string>ios-arm64_x86_64-simulator</string>
            <key>LibraryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>SupportedArchitectures</key>
            <array>
                <string>arm64</string>
                <string>x86_64</string>
            </array>
            <key>SupportedPlatform</key>
            <string>ios</string>
            <key>SupportedPlatformVariant</key>
            <string>simulator</string>
        </dict>
        <dict>
            <key>BinaryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>HeadersPath</key>
            <string>Headers</string>
            <key>LibraryIdentifier</key>
            <string>macos-arm64_x86_64</string>
            <key>LibraryPath</key>
            <string>libVifuMobileFFI.a</string>
            <key>SupportedArchitectures</key>
            <array>
                <string>arm64</string>
                <string>x86_64</string>
            </array>
            <key>SupportedPlatform</key>
            <string>macos</string>
        </dict>
    </array>
    <key>CFBundlePackageType</key>
    <string>XFWK</string>
    <key>XCFrameworkFormatVersion</key>
    <string>1.0</string>
</dict>
</plist>
PLIST
fi

# Fail before packaging if any advertised Apple platform slice is incomplete.
verify_architectures() {
    local library="$1"
    shift
    local actual
    actual="$(lipo -archs "$library")"
    for expected in "$@"; do
        if [[ " $actual " != *" $expected "* ]]; then
            echo "Missing $expected architecture in $library (found: $actual)" >&2
            exit 1
        fi
    done
}

if [[ "$MACOS_ONLY" == "true" ]]; then
    verify_architectures "$macos_dir/libVifuMobileFFI.a" arm64
elif [[ "$CI_ONLY" == "true" ]]; then
    verify_architectures "$simulator_dir/libVifuMobileFFI.a" arm64
    verify_architectures "$macos_dir/libVifuMobileFFI.a" arm64
else
    verify_architectures "$device_dir/libVifuMobileFFI.a" arm64
    verify_architectures "$simulator_dir/libVifuMobileFFI.a" arm64 x86_64
    verify_architectures "$macos_dir/libVifuMobileFFI.a" arm64 x86_64
fi

# Stable metadata keeps the SwiftPM checksum reproducible for the same inputs.
find "$xcframework" -exec touch -t 2001010000 {} +
(
    cd "$DIST_DIR"
    export COPYFILE_DISABLE=1
    find VifuMobileFFI.xcframework -print \
        | LC_ALL=C sort \
        | zip -X -q -@ VifuMobileFFI.xcframework.zip
)

checksum="$(swift package compute-checksum "$DIST_DIR/VifuMobileFFI.xcframework.zip")"
printf '%s  %s\n' \
    "$checksum" \
    "VifuMobileFFI.xcframework.zip" \
    > "$DIST_DIR/VifuMobileFFI.xcframework.zip.sha256"
mkdir -p "$(dirname "$LOCAL_XCFRAMEWORK")" "$LOCAL_XCFRAMEWORK"
rsync --archive --delete "$xcframework/" "$LOCAL_XCFRAMEWORK/"
printf 'Apple artifact: %s\nLocal SwiftPM artifact: %s\nSwiftPM checksum: %s\n' \
    "$DIST_DIR/VifuMobileFFI.xcframework.zip" \
    "$LOCAL_XCFRAMEWORK" \
    "$checksum"
