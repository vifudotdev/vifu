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
EXPECTED_MESSAGE="release: VifuGodot $RELEASE_TAG from Vifu"

if [[ "$RELEASE_REPOSITORY" != */* ]]; then
    echo "VIFUGODOT_RELEASE_REPOSITORY must be owner/name." >&2
    exit 64
fi

for command_name in base64 gh git jq; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "Missing required release command: $command_name" >&2
        exit 69
    fi
done

if [ -z "${GH_TOKEN:-}" ]; then
    echo "GH_TOKEN must contain a GitHub App installation token." >&2
    exit 69
fi

github_api() {
    gh api \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$@"
}

installation_repositories="$(github_api /installation/repositories 2>/dev/null)" || {
    echo "GH_TOKEN must be a GitHub App installation token." >&2
    exit 77
}
if ! jq -e --arg repository "$RELEASE_REPOSITORY" \
    'any(.repositories[]?; .full_name == $repository)' \
    >/dev/null <<<"$installation_repositories"
then
    echo "The GitHub App token cannot access $RELEASE_REPOSITORY." >&2
    exit 77
fi

if [ -n "$(git -C "$VIFU_ROOT" status --porcelain --untracked-files=all -- "$PACKAGE_PREFIX")" ]; then
    echo "Commit the VifuGodot package subtree before publishing it." >&2
    exit 65
fi

SOURCE_COMMIT="$(git -C "$VIFU_ROOT" rev-parse HEAD^{commit})"
if ! TAG_COMMIT="$(git -C "$VIFU_ROOT" rev-parse "refs/tags/$RELEASE_TAG^{commit}" 2>/dev/null)"; then
    echo "Vifu release tag does not exist in this checkout: $RELEASE_TAG" >&2
    exit 65
fi
if [ "$SOURCE_COMMIT" != "$TAG_COMMIT" ]; then
    echo "Vifu HEAD must be the commit referenced by $RELEASE_TAG." >&2
    exit 65
fi

PACKAGE_VERSION="${RELEASE_TAG#v}"
if ! grep -F "exact: \"$PACKAGE_VERSION\"" \
    "$VIFU_ROOT/$PACKAGE_PREFIX/Package.swift" >/dev/null
then
    echo "VifuGodot Package.swift must depend on Vifu $PACKAGE_VERSION." >&2
    exit 65
fi

SOURCE_TREE="$(git -C "$VIFU_ROOT" rev-parse "$SOURCE_COMMIT:$PACKAGE_PREFIX")"
COMMIT_MESSAGE="$EXPECTED_MESSAGE $SOURCE_COMMIT"

if github_api "repos/$RELEASE_REPOSITORY/git/ref/tags/$RELEASE_TAG" \
    >/dev/null 2>&1
then
    existing_commit="$(github_api "repos/$RELEASE_REPOSITORY/commits/$RELEASE_TAG")"
    existing_tree="$(jq -er '.commit.tree.sha' <<<"$existing_commit")"
    existing_verified="$(jq -er '.commit.verification.verified' <<<"$existing_commit")"
    if [ "$existing_tree" != "$SOURCE_TREE" ]; then
        echo "VifuGodot tag already exists with different package contents: $RELEASE_TAG" >&2
        exit 65
    fi
    if [ "$existing_verified" != "true" ]; then
        echo "VifuGodot tag does not resolve to a verified commit: $RELEASE_TAG" >&2
        exit 65
    fi
    if ! gh release view "$RELEASE_TAG" --repo "$RELEASE_REPOSITORY" >/dev/null 2>&1; then
        gh release create "$RELEASE_TAG" \
            --repo "$RELEASE_REPOSITORY" \
            --verify-tag \
            --generate-notes \
            --title "VifuGodot $RELEASE_TAG"
    fi
    echo "VifuGodot $RELEASE_TAG is already published from $SOURCE_COMMIT."
    exit 0
fi

parent_commit="$(
    github_api "repos/$RELEASE_REPOSITORY/git/ref/heads/main" \
        | jq -er '.object.sha'
)"
parent_object="$(github_api "repos/$RELEASE_REPOSITORY/git/commits/$parent_commit")"
parent_tree="$(jq -er '.tree.sha' <<<"$parent_object")"
parent_message="$(jq -er '.message' <<<"$parent_object")"
parent_verified="$(jq -er '.verification.verified' <<<"$parent_object")"

# A rerun after the branch update but before tag/release creation reuses the
# already-created signed snapshot instead of adding another commit.
if [ "$parent_tree" = "$SOURCE_TREE" ] \
    && [ "$parent_message" = "$COMMIT_MESSAGE" ] \
    && [ "$parent_verified" = "true" ]
then
    RELEASE_COMMIT="$parent_commit"
else
    tree_entries='[]'
    while IFS= read -r -d '' entry; do
        metadata="${entry%%$'\t'*}"
        path="${entry#*$'\t'}"
        mode="${metadata%% *}"
        metadata="${metadata#* }"
        object_type="${metadata%% *}"
        object_sha="${metadata##* }"

        if [ "$object_type" != "blob" ]; then
            echo "Unsupported VifuGodot tree entry: $path ($object_type)" >&2
            exit 65
        fi

        encoded="$(git -C "$VIFU_ROOT" cat-file blob "$object_sha" | base64 | tr -d '\n')"
        blob_payload="$(
            jq -cn --arg content "$encoded" \
                '{content: $content, encoding: "base64"}'
        )"
        uploaded_sha="$(
            printf '%s\n' "$blob_payload" \
                | github_api --method POST \
                    "repos/$RELEASE_REPOSITORY/git/blobs" \
                    --input - \
                | jq -er '.sha'
        )"
        if [ "$uploaded_sha" != "$object_sha" ]; then
            echo "GitHub returned a different blob for $path." >&2
            exit 70
        fi

        tree_entries="$(
            jq -c \
                --arg path "$path" \
                --arg mode "$mode" \
                --arg sha "$uploaded_sha" \
                '. + [{path: $path, mode: $mode, type: "blob", sha: $sha}]' \
                <<<"$tree_entries"
        )"
    done < <(git -C "$VIFU_ROOT" ls-tree -rz "$SOURCE_COMMIT:$PACKAGE_PREFIX")

    tree_payload="$(jq -cn --argjson tree "$tree_entries" '{tree: $tree}')"
    release_tree="$(
        printf '%s\n' "$tree_payload" \
            | github_api --method POST \
                "repos/$RELEASE_REPOSITORY/git/trees" \
                --input - \
            | jq -er '.sha'
    )"
    if [ "$release_tree" != "$SOURCE_TREE" ]; then
        echo "Published VifuGodot tree does not match the Vifu source subtree." >&2
        exit 70
    fi

    commit_payload="$(
        jq -cn \
            --arg message "$COMMIT_MESSAGE" \
            --arg tree "$release_tree" \
            --arg parent "$parent_commit" \
            '{message: $message, tree: $tree, parents: [$parent]}'
    )"
    commit_response="$(
        printf '%s\n' "$commit_payload" \
            | github_api --method POST \
                "repos/$RELEASE_REPOSITORY/git/commits" \
                --input -
    )"
    RELEASE_COMMIT="$(jq -er '.sha' <<<"$commit_response")"
    if [ "$(jq -er '.verification.verified' <<<"$commit_response")" != "true" ]; then
        echo "GitHub did not create a verified GitHub App commit." >&2
        exit 70
    fi

    github_api --method PATCH \
        "repos/$RELEASE_REPOSITORY/git/refs/heads/main" \
        -f "sha=$RELEASE_COMMIT" \
        -F force=false >/dev/null
fi

github_api --method POST \
    "repos/$RELEASE_REPOSITORY/git/refs" \
    -f "ref=refs/tags/$RELEASE_TAG" \
    -f "sha=$RELEASE_COMMIT" >/dev/null

published_commit="$(github_api "repos/$RELEASE_REPOSITORY/commits/$RELEASE_TAG")"
if [ "$(jq -er '.sha' <<<"$published_commit")" != "$RELEASE_COMMIT" ] \
    || [ "$(jq -er '.commit.tree.sha' <<<"$published_commit")" != "$SOURCE_TREE" ] \
    || [ "$(jq -er '.commit.verification.verified' <<<"$published_commit")" != "true" ]
then
    echo "Published VifuGodot tag failed commit verification." >&2
    exit 70
fi

gh release create "$RELEASE_TAG" \
    --repo "$RELEASE_REPOSITORY" \
    --verify-tag \
    --generate-notes \
    --title "VifuGodot $RELEASE_TAG"

printf 'Published VifuGodot %s from Vifu %s (%s)\n' \
    "$RELEASE_TAG" \
    "$SOURCE_COMMIT" \
    "$RELEASE_COMMIT"
