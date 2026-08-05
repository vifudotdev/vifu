#!/bin/bash

set -euo pipefail

usage() {
    echo "Usage: $0 <libgodot-checkout> <release-tag> <output-directory>" >&2
}

if [ "$#" -ne 3 ]; then
    usage
    exit 64
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
LIBGODOT_DIR="$(cd "$1" && pwd -P)"
RELEASE_TAG="$2"

if [[ ! "$RELEASE_TAG" =~ ^libgodot-[0-9]+\.[0-9]+\.[0-9]+-vifu\.[0-9]+$ ]]; then
    echo "Invalid release tag: $RELEASE_TAG" >&2
    exit 64
fi

mkdir -p "$3"
OUTPUT_DIR="$(cd "$3" && pwd -P)"
SOURCE_XCFRAMEWORK="$LIBGODOT_DIR/build/libgodot/release/libgodot.xcframework"

if [ -n "$(find "$OUTPUT_DIR" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
    echo "Output directory must be empty: $OUTPUT_DIR" >&2
    exit 73
fi

if ! git -C "$LIBGODOT_DIR" diff --quiet --ignore-submodules=dirty HEAD --; then
    echo "Refusing to package a libgodot checkout with tracked changes." >&2
    exit 65
fi

if ! git -C "$LIBGODOT_DIR/godot" diff --quiet HEAD --; then
    echo "Refusing to build a Godot checkout with tracked changes." >&2
    exit 65
fi

LIBGODOT_COMMIT="$(git -C "$LIBGODOT_DIR" rev-parse HEAD)"
GODOT_COMMIT="$(git -C "$LIBGODOT_DIR/godot" rev-parse HEAD)"
PINNED_GODOT_COMMIT="$(git -C "$LIBGODOT_DIR" ls-tree HEAD godot | awk '{ print $3 }')"

if [[ ! "$LIBGODOT_COMMIT" =~ ^[0-9a-f]{40}$ ]] \
    || [[ ! "$GODOT_COMMIT" =~ ^[0-9a-f]{40}$ ]]
then
    echo "Source checkouts must resolve to full Git commit SHAs." >&2
    exit 65
fi

if [ "$GODOT_COMMIT" != "$PINNED_GODOT_COMMIT" ]; then
    echo "Godot checkout does not match the libgodot gitlink." >&2
    echo "Expected: $PINNED_GODOT_COMMIT" >&2
    echo "Actual:   $GODOT_COMMIT" >&2
    exit 65
fi

for command_name in scons xcodebuild swift; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Missing required Apple build command: $command_name" >&2
        exit 69
    fi
done

echo "Building Apple libgodot from $LIBGODOT_COMMIT"
(
    cd "$LIBGODOT_DIR"
    ./build_libgodot.sh \
        --target ios \
        --target-arch arm64 \
        --release \
        --skip-host
    ./build_libgodot.sh \
        --target ios \
        --simulator \
        --target-arch arm64 \
        --release \
        --skip-host
    ./build_libgodot.sh \
        --target macos \
        --library-type static_library \
        --target-arch arm64 \
        --release \
        --skip-host
    ./build_libgodot_xcframework.sh --target template_release
)

if [ ! -d "$SOURCE_XCFRAMEWORK" ]; then
    echo "Build did not produce the release XCFramework: $SOURCE_XCFRAMEWORK" >&2
    exit 66
fi

"$SCRIPT_DIR/package-libgodot-apple.sh" "$SOURCE_XCFRAMEWORK" "$OUTPUT_DIR"
cp "$LIBGODOT_DIR/godot/LICENSE.txt" "$OUTPUT_DIR/Godot-LICENSE.txt"
cp "$LIBGODOT_DIR/godot/COPYRIGHT.txt" "$OUTPUT_DIR/Godot-COPYRIGHT.txt"

IOS_CHECKSUM="$(awk 'NR == 1 { print $1 }' "$OUTPUT_DIR/ios_libgodot.xcframework.zip.sha256")"
MACOS_CHECKSUM="$(awk 'NR == 1 { print $1 }' "$OUTPUT_DIR/mac_libgodot.xcframework.zip.sha256")"

cat > "$OUTPUT_DIR/libgodot-source.json" <<EOF
{
  "schemaVersion": 1,
  "releaseTag": "$RELEASE_TAG",
  "libgodot": {
    "repository": "https://github.com/vifudotdev/libgodot",
    "commit": "$LIBGODOT_COMMIT"
  },
  "godot": {
    "repository": "https://github.com/vifudotdev/godot",
    "commit": "$GODOT_COMMIT"
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
    "$OUTPUT_DIR" \
    "$RELEASE_TAG" \
    "$LIBGODOT_COMMIT"

echo "Prepared draft release assets in $OUTPUT_DIR"
