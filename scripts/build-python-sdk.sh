#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
DIST_DIR="${VIFU_PYTHON_DIST_DIR:-$TARGET_DIR/python-sdk}"
SOURCE_DIR="$REPO_ROOT/sdk/python/src/vifu"
CONSOLE_ASSETS_DIR="${VIFU_CONSOLE_ASSETS_DIR:-$REPO_ROOT/target/vifu-console-assets}"
OS_NAME="$(uname -s)"
IS_WINDOWS="false"

if [[ -z "${VIFU_CONSOLE_ASSETS_DIR:-}" ]]; then
    (
        cd "$REPO_ROOT"
        bun run build:console
    )
fi
if [[ ! -s "$CONSOLE_ASSETS_DIR/index.html" ]]; then
    echo "The Vifu Dashboard build did not produce $CONSOLE_ASSETS_DIR/index.html." >&2
    exit 1
fi
export VIFU_CONSOLE_ASSETS_DIR="$CONSOLE_ASSETS_DIR"
export VIFU_REQUIRE_CONSOLE_ASSETS=1

case "$OS_NAME" in
    Darwin)
        LIBRARY_NAME="libvifu_mobile_ffi.dylib"
        SERVER_NAME="vifu"
        ;;
    Linux)
        LIBRARY_NAME="libvifu_mobile_ffi.so"
        SERVER_NAME="vifu"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        LIBRARY_NAME="vifu_mobile_ffi.dll"
        SERVER_NAME="vifu.exe"
        IS_WINDOWS="true"
        ;;
    *)
        echo "The source build supports macOS, Linux, and Windows." >&2
        exit 1
        ;;
esac

cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    --release \
    -p vifu-mobile-ffi \
    --no-default-features
cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    --release \
    -p vifu \
    --no-default-features
cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    --locked \
    --release \
    -p vifu-mobile-ffi \
    --no-default-features \
    --bin uniffi-bindgen

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/vifu"
cp "$SOURCE_DIR"/*.py "$DIST_DIR/vifu/"
cp "$SOURCE_DIR/py.typed" "$DIST_DIR/vifu/"
mkdir -p "$DIST_DIR/vifu/integrations" "$DIST_DIR/vifu/_bin"
cp "$SOURCE_DIR/integrations"/*.py "$DIST_DIR/vifu/integrations/"
cp "$TARGET_DIR/release/$LIBRARY_NAME" "$DIST_DIR/vifu/$LIBRARY_NAME"
cp "$TARGET_DIR/release/$SERVER_NAME" "$DIST_DIR/vifu/_bin/$SERVER_NAME"
if [[ "$IS_WINDOWS" == "false" ]]; then
    chmod 0755 "$DIST_DIR/vifu/_bin/$SERVER_NAME"
fi
if [[ "$OS_NAME" == "Darwin" ]]; then
    install_name_tool \
        -id "@rpath/$LIBRARY_NAME" \
        "$DIST_DIR/vifu/$LIBRARY_NAME"
fi
"$TARGET_DIR/release/uniffi-bindgen" generate \
    --library "$TARGET_DIR/release/$LIBRARY_NAME" \
    --language python \
    --out-dir "$DIST_DIR/vifu" \
    --config "$REPO_ROOT/crates/vifu-mobile-ffi/uniffi.toml"

printf 'Built Vifu Python SDK: %s\n' "$DIST_DIR"
