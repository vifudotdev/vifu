#!/bin/bash

set -euo pipefail

usage() {
    echo "Usage: $0 <libgodot.xcframework> <output-directory>" >&2
}

if [ "$#" -ne 2 ]; then
    usage
    exit 64
fi

SOURCE_XCFRAMEWORK="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
mkdir -p "$2"
OUTPUT_DIR="$(cd "$2" && pwd -P)"
WORK_DIR="$OUTPUT_DIR/.libgodot-package-work"

if [ ! -f "$SOURCE_XCFRAMEWORK/Info.plist" ]; then
    echo "Missing XCFramework Info.plist: $SOURCE_XCFRAMEWORK" >&2
    exit 66
fi

IOS_DEVICE_FRAMEWORK="$SOURCE_XCFRAMEWORK/ios-arm64/libgodot.framework"
IOS_SIMULATOR_FRAMEWORK="$SOURCE_XCFRAMEWORK/ios-arm64-simulator/libgodot.framework"
MACOS_FRAMEWORK="$SOURCE_XCFRAMEWORK/macos-arm64/libgodot.framework"

for framework in \
    "$IOS_DEVICE_FRAMEWORK" \
    "$IOS_SIMULATOR_FRAMEWORK" \
    "$MACOS_FRAMEWORK"
do
    if [ ! -f "$framework/Info.plist" ] \
        && [ ! -f "$framework/Resources/Info.plist" ]
    then
        echo "Missing required libgodot framework slice: $framework" >&2
        exit 66
    fi
done

verify_release_binary() {
    local binary="$1"
    if ! LC_ALL=C strings "$binary" \
        | awk '$0 == "template_release" { found = 1 } END { exit !found }'
    then
        echo "Refusing to package a non-release libgodot binary: $binary" >&2
        exit 65
    fi
}

verify_architecture() {
    local binary="$1"
    local expected="$2"
    local architectures
    architectures="$(lipo -archs "$binary")"
    case " $architectures " in
        *" $expected "*) ;;
        *)
            echo "Missing $expected architecture in $binary: $architectures" >&2
            exit 65
            ;;
    esac
}

IOS_DEVICE_BINARY="$IOS_DEVICE_FRAMEWORK/libgodot"
IOS_SIMULATOR_BINARY="$IOS_SIMULATOR_FRAMEWORK/libgodot"
MACOS_BINARY="$MACOS_FRAMEWORK/Versions/A/libgodot"

verify_release_binary "$IOS_DEVICE_BINARY"
verify_release_binary "$IOS_SIMULATOR_BINARY"
verify_release_binary "$MACOS_BINARY"
verify_architecture "$IOS_DEVICE_BINARY" arm64
verify_architecture "$IOS_SIMULATOR_BINARY" arm64
verify_architecture "$MACOS_BINARY" arm64

case "$WORK_DIR" in
    "$OUTPUT_DIR"/*) ;;
    *)
        echo "Refusing unsafe work directory: $WORK_DIR" >&2
        exit 64
        ;;
esac

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR"

create_ios_xcframework() {
    local output="$WORK_DIR/ios_libgodot.xcframework"
    local args=(
        -create-xcframework
        -framework "$IOS_DEVICE_FRAMEWORK"
    )

    if [ -d "$SOURCE_XCFRAMEWORK/ios-arm64/dSYMs/libgodot.framework.dSYM" ]; then
        args+=(
            -debug-symbols
            "$SOURCE_XCFRAMEWORK/ios-arm64/dSYMs/libgodot.framework.dSYM"
        )
    fi
    args+=(-framework "$IOS_SIMULATOR_FRAMEWORK")
    if [ -d "$SOURCE_XCFRAMEWORK/ios-arm64-simulator/dSYMs/libgodot.framework.dSYM" ]; then
        args+=(
            -debug-symbols
            "$SOURCE_XCFRAMEWORK/ios-arm64-simulator/dSYMs/libgodot.framework.dSYM"
        )
    fi
    args+=(-output "$output")

    xcodebuild "${args[@]}"
}

create_macos_xcframework() {
    local output="$WORK_DIR/mac_libgodot.xcframework"
    local args=(
        -create-xcframework
        -framework "$MACOS_FRAMEWORK"
    )

    if [ -d "$SOURCE_XCFRAMEWORK/macos-arm64/dSYMs/libgodot.framework.dSYM" ]; then
        args+=(
            -debug-symbols
            "$SOURCE_XCFRAMEWORK/macos-arm64/dSYMs/libgodot.framework.dSYM"
        )
    fi
    args+=(-output "$output")

    xcodebuild "${args[@]}"
}

archive_xcframework() {
    local name="$1"
    local archive="$OUTPUT_DIR/$name.xcframework.zip"
    local normalized_mtime="20010101""0000"

    find "$WORK_DIR/$name.xcframework" -exec touch -t "$normalized_mtime" {} +
    rm -f "$archive" "$archive.sha256"
    (
        cd "$WORK_DIR"
        find "$name.xcframework" -print \
            | LC_ALL=C sort \
            | zip -X -y -q "$archive" -@
    )

    local checksum
    checksum="$(swift package compute-checksum "$archive")"
    printf '%s  %s\n' "$checksum" "$(basename "$archive")" > "$archive.sha256"
    printf '%s checksum: %s\n' "$(basename "$archive")" "$checksum"
}

create_ios_xcframework
create_macos_xcframework
archive_xcframework ios_libgodot
archive_xcframework mac_libgodot
