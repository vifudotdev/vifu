#!/bin/bash

set -euo pipefail

usage() {
    echo "Usage: $0 <asset-directory> <release-tag>" >&2
}

if [ "$#" -ne 2 ]; then
    usage
    exit 64
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
VIFU_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
ASSET_DIR="$(cd "$1" && pwd -P)"
RELEASE_TAG="$2"
RELEASE_REPOSITORY="${VIFU_RELEASE_REPOSITORY:-vifudotdev/vifu}"
MANIFEST="$ASSET_DIR/libgodot-source.json"

"$SCRIPT_DIR/verify-libgodot-apple-release.sh" "$ASSET_DIR" "$RELEASE_TAG"

if gh release view "$RELEASE_TAG" --repo "$RELEASE_REPOSITORY" >/dev/null 2>&1; then
    echo "Release already exists and will not be overwritten: $RELEASE_TAG" >&2
    exit 65
fi

VIFU_COMMIT="$(git -C "$VIFU_ROOT" rev-parse HEAD)"
LIBGODOT_COMMIT="$(plutil -extract libgodot.commit raw -o - "$MANIFEST")"
GODOT_COMMIT="$(plutil -extract godot.commit raw -o - "$MANIFEST")"

gh api "repos/$RELEASE_REPOSITORY/commits/$VIFU_COMMIT" >/dev/null
gh api "repos/vifudotdev/libgodot/git/commits/$LIBGODOT_COMMIT" >/dev/null
gh api "repos/vifudotdev/godot/git/commits/$GODOT_COMMIT" >/dev/null

NOTES_DIR="$(mktemp -d "$ASSET_DIR/.libgodot-draft.XXXXXX")"
case "$NOTES_DIR" in
    "$ASSET_DIR"/*) ;;
    *)
        echo "Refusing unsafe draft directory: $NOTES_DIR" >&2
        exit 64
        ;;
esac

cleanup() {
    rm -rf "$NOTES_DIR"
}
trap cleanup EXIT

cat > "$NOTES_DIR/notes.md" <<EOF
Prebuilt Apple libgodot runtime for VifuGodot.

- libgodot commit: \`$LIBGODOT_COMMIT\`
- Godot commit: \`$GODOT_COMMIT\`

This draft is published only after the release verification workflow validates
the source manifest, SwiftPM checksums, framework slices, architectures, and
release binaries.
EOF

gh release create "$RELEASE_TAG" \
    "$ASSET_DIR/ios_libgodot.xcframework.zip" \
    "$ASSET_DIR/ios_libgodot.xcframework.zip.sha256" \
    "$ASSET_DIR/mac_libgodot.xcframework.zip" \
    "$ASSET_DIR/mac_libgodot.xcframework.zip.sha256" \
    "$ASSET_DIR/Godot-LICENSE.txt" \
    "$ASSET_DIR/Godot-COPYRIGHT.txt" \
    "$ASSET_DIR/libgodot-source.json" \
    --repo "$RELEASE_REPOSITORY" \
    --target "$VIFU_COMMIT" \
    --title "Vifu $RELEASE_TAG" \
    --notes-file "$NOTES_DIR/notes.md" \
    --draft

echo "Created draft release $RELEASE_TAG in $RELEASE_REPOSITORY"
echo "Run the Release Vifu libgodot binaries workflow to verify and publish it."
