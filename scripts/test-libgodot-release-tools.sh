#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
VIFU_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
mkdir -p "$VIFU_ROOT/.build"
WORK_DIR="$(mktemp -d "$VIFU_ROOT/.build/libgodot-release-test.XXXXXX")"

case "$WORK_DIR" in
    "$VIFU_ROOT/.build"/*) ;;
    *)
        echo "Refusing unsafe test directory: $WORK_DIR" >&2
        exit 64
        ;;
esac

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

cat > "$WORK_DIR/libgodot.c" <<'EOF'
__attribute__((visibility("default")))
const char *vifu_libgodot_build_marker(void) {
    return "template_release";
}
EOF

write_framework_plist() {
    local plist="$1"
    plutil -create xml1 "$plist"
    plutil -insert CFBundleExecutable -string libgodot "$plist"
    plutil -insert CFBundleIdentifier -string dev.vifu.libgodot.fixture "$plist"
    plutil -insert CFBundleName -string libgodot "$plist"
    plutil -insert CFBundlePackageType -string FMWK "$plist"
}

DEVICE_FRAMEWORK="$WORK_DIR/device/libgodot.framework"
SIMULATOR_FRAMEWORK="$WORK_DIR/simulator/libgodot.framework"
MACOS_FRAMEWORK="$WORK_DIR/macos/libgodot.framework"
mkdir -p "$DEVICE_FRAMEWORK" "$SIMULATOR_FRAMEWORK" "$MACOS_FRAMEWORK/Versions/A/Resources"

IPHONEOS_SDK="$(xcrun --sdk iphoneos --show-sdk-path)"
SIMULATOR_SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"
MACOS_SDK="$(xcrun --sdk macosx --show-sdk-path)"

xcrun clang \
    -target arm64-apple-ios17.0 \
    -isysroot "$IPHONEOS_SDK" \
    -dynamiclib \
    -install_name @rpath/libgodot.framework/libgodot \
    "$WORK_DIR/libgodot.c" \
    -o "$DEVICE_FRAMEWORK/libgodot"
write_framework_plist "$DEVICE_FRAMEWORK/Info.plist"

xcrun clang \
    -target arm64-apple-ios17.0-simulator \
    -isysroot "$SIMULATOR_SDK" \
    -dynamiclib \
    -install_name @rpath/libgodot.framework/libgodot \
    "$WORK_DIR/libgodot.c" \
    -o "$SIMULATOR_FRAMEWORK/libgodot"
write_framework_plist "$SIMULATOR_FRAMEWORK/Info.plist"

xcrun clang \
    -target arm64-apple-macos14.0 \
    -isysroot "$MACOS_SDK" \
    -dynamiclib \
    -install_name @rpath/libgodot.framework/Versions/A/libgodot \
    "$WORK_DIR/libgodot.c" \
    -o "$MACOS_FRAMEWORK/Versions/A/libgodot"
write_framework_plist "$MACOS_FRAMEWORK/Versions/A/Resources/Info.plist"
ln -s A "$MACOS_FRAMEWORK/Versions/Current"
ln -s Versions/Current/libgodot "$MACOS_FRAMEWORK/libgodot"
ln -s Versions/Current/Resources "$MACOS_FRAMEWORK/Resources"

xcodebuild -create-xcframework \
    -framework "$DEVICE_FRAMEWORK" \
    -framework "$SIMULATOR_FRAMEWORK" \
    -framework "$MACOS_FRAMEWORK" \
    -output "$WORK_DIR/libgodot.xcframework" >/dev/null

ASSET_DIR="$WORK_DIR/assets"
"$SCRIPT_DIR/package-libgodot-apple.sh" \
    "$WORK_DIR/libgodot.xcframework" \
    "$ASSET_DIR"
printf 'fixture license\n' > "$ASSET_DIR/Godot-LICENSE.txt"
printf 'fixture copyright\n' > "$ASSET_DIR/Godot-COPYRIGHT.txt"

IOS_CHECKSUM="$(awk 'NR == 1 { print $1 }' "$ASSET_DIR/ios_libgodot.xcframework.zip.sha256")"
MACOS_CHECKSUM="$(awk 'NR == 1 { print $1 }' "$ASSET_DIR/mac_libgodot.xcframework.zip.sha256")"

cat > "$ASSET_DIR/libgodot-source.json" <<EOF
{
  "schemaVersion": 1,
  "releaseTag": "libgodot-4.5.1-vifu.1",
  "libgodot": {
    "repository": "https://github.com/vifudotdev/libgodot",
    "commit": "1111111111111111111111111111111111111111"
  },
  "godot": {
    "repository": "https://github.com/vifudotdev/godot",
    "commit": "2222222222222222222222222222222222222222"
  },
  "artifacts": {
    "ios": {
      "name": "ios_libgodot.xcframework.zip",
      "checksum": "$IOS_CHECKSUM"
    },
    "macos": {
      "name": "mac_libgodot.xcframework.zip",
      "checksum": "$MACOS_CHECKSUM"
    }
  }
}
EOF

"$SCRIPT_DIR/verify-libgodot-apple-release.sh" \
    "$ASSET_DIR" \
    libgodot-4.5.1-vifu.1 \
    1111111111111111111111111111111111111111

printf '%064d  ios_libgodot.xcframework.zip\n' 0 \
    > "$ASSET_DIR/ios_libgodot.xcframework.zip.sha256"
if "$SCRIPT_DIR/verify-libgodot-apple-release.sh" \
    "$ASSET_DIR" \
    libgodot-4.5.1-vifu.1 \
    1111111111111111111111111111111111111111 >/dev/null 2>&1
then
    echo "Verifier accepted a tampered checksum." >&2
    exit 1
fi

echo "libgodot release tool tests passed"
