#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON_BIN="${PYTHON_BIN:-python3}"
MACHINE="$(uname -m)"

case "$(uname -s)" in
    Darwin)
        case "$MACHINE" in
            arm64|aarch64)
                DEFAULT_PLATFORM_TAG="macosx_11_0_arm64"
                export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
                ;;
            x86_64)
                DEFAULT_PLATFORM_TAG="macosx_10_12_x86_64"
                export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.12}"
                ;;
            *)
                echo "Unsupported macOS architecture: $MACHINE" >&2
                exit 1
                ;;
        esac
        ;;
    Linux)
        case "$MACHINE" in
            x86_64|aarch64)
                DEFAULT_PLATFORM_TAG="linux_${MACHINE}"
                ;;
            *)
                echo "Unsupported Linux architecture: $MACHINE" >&2
                exit 1
                ;;
        esac
        ;;
    MINGW*|MSYS*|CYGWIN*)
        case "$MACHINE" in
            x86_64)
                DEFAULT_PLATFORM_TAG="win_amd64"
                ;;
            arm64|aarch64)
                DEFAULT_PLATFORM_TAG="win_arm64"
                ;;
            *)
                echo "Unsupported Windows architecture: $MACHINE" >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "The Python wheel build supports macOS, Linux, and Windows." >&2
        exit 1
        ;;
esac

PLATFORM_TAG="${VIFU_PYTHON_WHEEL_PLATFORM:-$DEFAULT_PLATFORM_TAG}"
PACKAGE_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}/python-sdk/vifu"
OUTPUT_DIR="${VIFU_PYTHON_WHEEL_DIR:-$REPO_ROOT/target/python-wheel}"

"$REPO_ROOT/scripts/build-python-sdk.sh"
"$PYTHON_BIN" "$REPO_ROOT/scripts/build-python-wheel.py" \
    --package-dir "$PACKAGE_DIR" \
    --output-dir "$OUTPUT_DIR" \
    --platform-tag "$PLATFORM_TAG"
