#!/bin/bash

set -euo pipefail

usage() {
    echo "Usage: $0 <asset-directory> [expected-release-tag] [expected-libgodot-commit]" >&2
}

if [ "$#" -lt 1 ] || [ "$#" -gt 3 ]; then
    usage
    exit 64
fi

ASSET_DIR="$(cd "$1" && pwd -P)"
EXPECTED_RELEASE_TAG="${2:-}"
EXPECTED_LIBGODOT_COMMIT="${3:-}"
MANIFEST="$ASSET_DIR/libgodot-source.json"

if [ -n "$EXPECTED_RELEASE_TAG" ] \
    && [[ ! "$EXPECTED_RELEASE_TAG" =~ ^libgodot-[0-9]+\.[0-9]+\.[0-9]+-vifu\.[0-9]+$ ]]
then
    echo "Invalid release tag: $EXPECTED_RELEASE_TAG" >&2
    exit 64
fi

if [ -n "$EXPECTED_LIBGODOT_COMMIT" ] \
    && [[ ! "$EXPECTED_LIBGODOT_COMMIT" =~ ^[0-9a-f]{40}$ ]]
then
    echo "Expected libgodot commit must be a full lowercase Git SHA." >&2
    exit 64
fi

required_assets=(
    ios_libgodot.xcframework.zip
    ios_libgodot.xcframework.zip.sha256
    mac_libgodot.xcframework.zip
    mac_libgodot.xcframework.zip.sha256
    Godot-LICENSE.txt
    Godot-COPYRIGHT.txt
    libgodot-source.json
)

for asset in "${required_assets[@]}"; do
    if [ ! -s "$ASSET_DIR/$asset" ]; then
        echo "Missing or empty release asset: $asset" >&2
        exit 66
    fi
done

read_manifest_value() {
    plutil -extract "$1" raw -o - "$MANIFEST"
}

SCHEMA_VERSION="$(read_manifest_value schemaVersion)"
RELEASE_TAG="$(read_manifest_value releaseTag)"
LIBGODOT_REPOSITORY="$(read_manifest_value libgodot.repository)"
LIBGODOT_COMMIT="$(read_manifest_value libgodot.commit)"
GODOT_REPOSITORY="$(read_manifest_value godot.repository)"
GODOT_COMMIT="$(read_manifest_value godot.commit)"

if [ "$SCHEMA_VERSION" != "1" ]; then
    echo "Unsupported libgodot release manifest schema: $SCHEMA_VERSION" >&2
    exit 65
fi

if [[ ! "$RELEASE_TAG" =~ ^libgodot-[0-9]+\.[0-9]+\.[0-9]+-vifu\.[0-9]+$ ]]; then
    echo "Invalid release tag in manifest: $RELEASE_TAG" >&2
    exit 65
fi

if [ -n "$EXPECTED_RELEASE_TAG" ] && [ "$RELEASE_TAG" != "$EXPECTED_RELEASE_TAG" ]; then
    echo "Release tag mismatch: expected $EXPECTED_RELEASE_TAG, found $RELEASE_TAG" >&2
    exit 65
fi

if [ "$LIBGODOT_REPOSITORY" != "https://github.com/vifudotdev/libgodot" ]; then
    echo "Unexpected libgodot repository: $LIBGODOT_REPOSITORY" >&2
    exit 65
fi

if [ "$GODOT_REPOSITORY" != "https://github.com/vifudotdev/godot" ]; then
    echo "Unexpected Godot repository: $GODOT_REPOSITORY" >&2
    exit 65
fi

if [[ ! "$LIBGODOT_COMMIT" =~ ^[0-9a-f]{40}$ ]] \
    || [[ ! "$GODOT_COMMIT" =~ ^[0-9a-f]{40}$ ]]
then
    echo "Release manifest must contain full lowercase source commit SHAs." >&2
    exit 65
fi

if [ -n "$EXPECTED_LIBGODOT_COMMIT" ] \
    && [ "$LIBGODOT_COMMIT" != "$EXPECTED_LIBGODOT_COMMIT" ]
then
    echo "libgodot commit mismatch: expected $EXPECTED_LIBGODOT_COMMIT, found $LIBGODOT_COMMIT" >&2
    exit 65
fi

verify_checksum() {
    local archive_name="$1"
    local manifest_key="$2"
    local checksum_file="$ASSET_DIR/$archive_name.sha256"
    local recorded_checksum
    local recorded_name
    local manifest_name
    local manifest_checksum
    local computed_checksum

    if [ "$(wc -l < "$checksum_file" | tr -d ' ')" != "1" ]; then
        echo "Checksum file must contain exactly one line: $(basename "$checksum_file")" >&2
        exit 65
    fi

    read -r recorded_checksum recorded_name < "$checksum_file"
    manifest_name="$(read_manifest_value "artifacts.$manifest_key.name")"
    manifest_checksum="$(read_manifest_value "artifacts.$manifest_key.checksum")"

    if [[ ! "$recorded_checksum" =~ ^[0-9a-f]{64}$ ]] \
        || [ "$recorded_name" != "$archive_name" ]
    then
        echo "Invalid checksum record: $(basename "$checksum_file")" >&2
        exit 65
    fi

    if [ "$manifest_name" != "$archive_name" ] \
        || [ "$manifest_checksum" != "$recorded_checksum" ]
    then
        echo "Manifest checksum does not match $archive_name" >&2
        exit 65
    fi

    computed_checksum="$(swift package compute-checksum "$ASSET_DIR/$archive_name")"
    if [ "$computed_checksum" != "$recorded_checksum" ]; then
        echo "Archive checksum does not match $archive_name" >&2
        exit 65
    fi
}

verify_archive_entries() {
    local archive_name="$1"
    local expected_root="$2"

    if ! zipinfo -1 "$ASSET_DIR/$archive_name" \
        | awk -v root="$expected_root/" '
            BEGIN { found = 0 }
            index($0, "\\") || $0 ~ /^\// || $0 ~ /(^|\/)\.\.($|\/)/ { exit 1 }
            index($0, root) != 1 { exit 1 }
            { found = 1 }
            END { if (!found) exit 1 }
        '
    then
        echo "Unsafe or unexpected paths in archive: $archive_name" >&2
        exit 65
    fi
}

verify_release_binary() {
    local binary="$1"
    if ! LC_ALL=C strings "$binary" \
        | awk '$0 == "template_release" { found = 1 } END { exit !found }'
    then
        echo "Non-release libgodot binary: $binary" >&2
        exit 65
    fi
}

verify_architecture() {
    local binary="$1"
    local expected="$2"
    local architectures
    architectures="$(lipo -archs "$binary")"
    if [ "$architectures" != "$expected" ]; then
        echo "Expected only $expected in $binary, found: $architectures" >&2
        exit 65
    fi
}

verify_build_version() {
    local binary="$1"
    local expected_platform="$2"
    local expected_minos="$3"
    local build_info
    local platform
    local minos

    build_info="$(xcrun vtool -show-build "$binary")"
    platform="$(awk '$1 == "platform" { print $2; exit }' <<< "$build_info")"
    minos="$(awk '$1 == "minos" { print $2; exit }' <<< "$build_info")"

    if [ "$platform" != "$expected_platform" ]; then
        echo "Expected platform $expected_platform in $binary, found: ${platform:-missing}" >&2
        exit 65
    fi

    if [ "$minos" != "$expected_minos" ]; then
        echo "Expected minimum OS $expected_minos in $binary, found: ${minos:-missing}" >&2
        exit 65
    fi
}

verify_checksum ios_libgodot.xcframework.zip ios
verify_checksum mac_libgodot.xcframework.zip macos
verify_archive_entries ios_libgodot.xcframework.zip ios_libgodot.xcframework
verify_archive_entries mac_libgodot.xcframework.zip mac_libgodot.xcframework

WORK_DIR="$(mktemp -d "$ASSET_DIR/.libgodot-verify.XXXXXX")"
case "$WORK_DIR" in
    "$ASSET_DIR"/*) ;;
    *)
        echo "Refusing unsafe verification directory: $WORK_DIR" >&2
        exit 64
        ;;
esac

cleanup() {
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

unzip -q "$ASSET_DIR/ios_libgodot.xcframework.zip" -d "$WORK_DIR"
unzip -q "$ASSET_DIR/mac_libgodot.xcframework.zip" -d "$WORK_DIR"

IOS_DEVICE_BINARY="$WORK_DIR/ios_libgodot.xcframework/ios-arm64/libgodot.framework/libgodot"
IOS_SIMULATOR_BINARY="$WORK_DIR/ios_libgodot.xcframework/ios-arm64-simulator/libgodot.framework/libgodot"
MACOS_BINARY="$WORK_DIR/mac_libgodot.xcframework/macos-arm64/libgodot.framework/Versions/A/libgodot"

for binary in "$IOS_DEVICE_BINARY" "$IOS_SIMULATOR_BINARY" "$MACOS_BINARY"; do
    if [ ! -f "$binary" ]; then
        echo "Missing framework binary: $binary" >&2
        exit 66
    fi
    verify_release_binary "$binary"
    verify_architecture "$binary" arm64
done

verify_build_version "$IOS_DEVICE_BINARY" IOS 15.0
verify_build_version "$IOS_SIMULATOR_BINARY" IOSSIMULATOR 15.0
verify_build_version "$MACOS_BINARY" MACOS 14.0

echo "Verified Vifu Apple libgodot release assets for $RELEASE_TAG"
echo "libgodot commit: $LIBGODOT_COMMIT"
echo "Godot commit: $GODOT_COMMIT"
