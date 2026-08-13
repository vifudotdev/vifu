#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
DIST_DIR="${VIFU_PYTHON_DIST_DIR:-$TARGET_DIR/python-sdk}"
SOURCE_DIR="$REPO_ROOT/sdk/python/src/vifu"

case "$(uname -s)" in
    Darwin)
        LIBRARY_NAME="libvifu_mobile_ffi.dylib"
        ;;
    Linux)
        LIBRARY_NAME="libvifu_mobile_ffi.so"
        ;;
    *)
        echo "The source build currently supports macOS and Linux." >&2
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
    -p vifu-mobile-ffi \
    --no-default-features \
    --bin uniffi-bindgen

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/vifu"
cp "$SOURCE_DIR"/*.py "$DIST_DIR/vifu/"
cp "$SOURCE_DIR/py.typed" "$DIST_DIR/vifu/"
cp "$TARGET_DIR/release/$LIBRARY_NAME" "$DIST_DIR/vifu/$LIBRARY_NAME"
"$TARGET_DIR/debug/uniffi-bindgen" generate \
    --library "$TARGET_DIR/release/$LIBRARY_NAME" \
    --language python \
    --out-dir "$DIST_DIR/vifu" \
    --config "$REPO_ROOT/crates/vifu-mobile-ffi/uniffi.toml"

printf 'Built Vifu Python SDK: %s\n' "$DIST_DIR"
