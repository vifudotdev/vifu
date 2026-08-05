#!/usr/bin/env bash

set -euo pipefail

usage() {
    echo "Usage: $0 <release-tag>" >&2
}

if [ "$#" -ne 1 ]; then
    usage
    exit 64
fi

RELEASE_TAG="$1"
if [[ ! "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
    echo "Release tag must be a semantic version beginning with v." >&2
    exit 64
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
VIFU_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
PACKAGE_PREFIX="integrations/godot/apple"
RELEASE_REPOSITORY="${VIFUGODOT_RELEASE_REPOSITORY:-vifudotdev/VifuGodot}"
RELEASE_URL="https://github.com/${RELEASE_REPOSITORY}.git"

if [ -n "$(git -C "$VIFU_ROOT" status --porcelain --untracked-files=all -- "$PACKAGE_PREFIX")" ]; then
    echo "Commit the VifuGodot package subtree before publishing it." >&2
    exit 65
fi

git -C "$VIFU_ROOT" fetch origin main
local_head="$(git -C "$VIFU_ROOT" rev-parse HEAD)"
remote_head="$(git -C "$VIFU_ROOT" rev-parse origin/main)"
if [ "$local_head" != "$remote_head" ]; then
    echo "Vifu HEAD must match the published origin/main commit." >&2
    exit 65
fi

if git ls-remote --exit-code --tags "$RELEASE_URL" "refs/tags/$RELEASE_TAG" >/dev/null 2>&1; then
    echo "VifuGodot tag already exists: $RELEASE_TAG" >&2
    exit 65
fi

split_commit="$(git -C "$VIFU_ROOT" subtree split --prefix "$PACKAGE_PREFIX" HEAD)"
git -C "$VIFU_ROOT" push --atomic "$RELEASE_URL" \
    "$split_commit:refs/heads/main" \
    "$split_commit:refs/tags/$RELEASE_TAG"

gh release create "$RELEASE_TAG" \
    --repo "$RELEASE_REPOSITORY" \
    --verify-tag \
    --generate-notes \
    --title "VifuGodot $RELEASE_TAG"

printf 'Published VifuGodot %s from Vifu %s (%s)\n' \
    "$RELEASE_TAG" \
    "$local_head" \
    "$split_commit"
