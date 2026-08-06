#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
VIFU_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
PACKAGE_VERSION="$(sed -n '/vifudotdev\/vifu\.git/ s/.*exact: "\([^"]*\)".*/\1/p' "$VIFU_ROOT/integrations/godot/apple/Package.swift")"
if [[ ! "$PACKAGE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
    echo "VifuGodot Package.swift must pin a Vifu semantic version." >&2
    exit 64
fi
RELEASE_TAG="v$PACKAGE_VERSION"
mkdir -p "$VIFU_ROOT/.build"
WORK_DIR="$(mktemp -d "$VIFU_ROOT/.build/vifugodot-publish-test.XXXXXX")"

case "$WORK_DIR" in
    "$VIFU_ROOT/.build"/*) ;;
    *)
        echo "Refusing unsafe test directory: $WORK_DIR" >&2
        exit 64
        ;;
esac

cleanup() {
    rm -rf -- "$WORK_DIR"
}
trap cleanup EXIT

SOURCE_DIR="$WORK_DIR/source"
FAKE_BIN="$WORK_DIR/bin"
STATE_DIR="$WORK_DIR/state"
mkdir -p \
    "$SOURCE_DIR/integrations/godot/apple" \
    "$SOURCE_DIR/scripts" \
    "$FAKE_BIN" \
    "$STATE_DIR"

git -C "$VIFU_ROOT" archive HEAD:integrations/godot/apple \
    | tar -x -C "$SOURCE_DIR/integrations/godot/apple"
cp "$SCRIPT_DIR/publish-vifugodot.sh" "$SOURCE_DIR/scripts/publish-vifugodot.sh"

git -C "$SOURCE_DIR" init -q
git -C "$SOURCE_DIR" config user.name "Vifu release test"
git -C "$SOURCE_DIR" config user.email "release-test@vifu.dev"
git -C "$SOURCE_DIR" config commit.gpgsign false
git -C "$SOURCE_DIR" config tag.gpgSign false
git -C "$SOURCE_DIR" config core.hooksPath /dev/null
git -C "$SOURCE_DIR" add integrations scripts/publish-vifugodot.sh
git -C "$SOURCE_DIR" commit -q -m "test: VifuGodot release source"
git -C "$SOURCE_DIR" tag "$RELEASE_TAG"

SOURCE_COMMIT="$(git -C "$SOURCE_DIR" rev-parse HEAD)"
SOURCE_TREE="$(git -C "$SOURCE_DIR" rev-parse HEAD:integrations/godot/apple)"
PARENT_COMMIT="1111111111111111111111111111111111111111"
RELEASE_COMMIT="2222222222222222222222222222222222222222"
TARGET_REPOSITORY="vifudotdev/VifuGodot"

cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash

set -euo pipefail

subcommand="$1"
shift

if [ "$subcommand" = "release" ]; then
    operation="$1"
    shift
    case "$operation" in
        view)
            if [ -f "$FAKE_STATE_DIR/release-created" ]; then
                jq -cn --arg tag "$FAKE_RELEASE_TAG" '{tagName: $tag}'
                exit 0
            fi
            exit 1
            ;;
        create)
            [ "$1" = "$FAKE_RELEASE_TAG" ]
            printf 'created\n' > "$FAKE_STATE_DIR/release-created"
            printf 'https://github.com/%s/releases/tag/%s\n' "$FAKE_TARGET_REPOSITORY" "$FAKE_RELEASE_TAG"
            exit 0
            ;;
        *)
            echo "Unexpected fake gh release operation: $operation" >&2
            exit 2
            ;;
    esac
fi

if [ "$subcommand" != "api" ]; then
    echo "Unexpected fake gh subcommand: $subcommand" >&2
    exit 2
fi

method=GET
endpoint=""
input=""
fields=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        -H)
            shift 2
            ;;
        --method)
            method="$2"
            shift 2
            ;;
        --input)
            input="$2"
            shift 2
            ;;
        -f|-F)
            fields+=("$2")
            shift 2
            ;;
        *)
            if [ -z "$endpoint" ]; then
                endpoint="$1"
                shift
            else
                echo "Unexpected fake gh api argument: $1" >&2
                exit 2
            fi
            ;;
    esac
done

payload=""
if [ "$input" = "-" ]; then
    payload="$(</dev/stdin)"
fi

field_value() {
    local wanted="$1"
    local field
    for field in "${fields[@]}"; do
        if [[ "$field" == "$wanted="* ]]; then
            printf '%s\n' "${field#*=}"
            return 0
        fi
    done
    return 1
}

case "$method:$endpoint" in
    "GET:/installation/repositories")
        jq -cn --arg repository "$FAKE_TARGET_REPOSITORY" \
            '{repositories: [{full_name: $repository}]}'
        ;;
    "GET:repos/$FAKE_TARGET_REPOSITORY/git/ref/tags/$FAKE_RELEASE_TAG")
        if [ ! -f "$FAKE_STATE_DIR/tag-created" ]; then
            exit 1
        fi
        jq -cn --arg sha "$FAKE_RELEASE_COMMIT" \
            --arg tag "$FAKE_RELEASE_TAG" \
            '{ref: ("refs/tags/" + $tag), object: {type: "commit", sha: $sha}}'
        ;;
    "GET:repos/$FAKE_TARGET_REPOSITORY/git/ref/heads/main")
        jq -cn --arg sha "$FAKE_PARENT_COMMIT" \
            '{ref: "refs/heads/main", object: {type: "commit", sha: $sha}}'
        ;;
    "GET:repos/$FAKE_TARGET_REPOSITORY/git/commits/$FAKE_PARENT_COMMIT")
        jq -cn \
            --arg sha "$FAKE_PARENT_COMMIT" \
            '{sha: $sha, message: "previous release", tree: {sha: "3333333333333333333333333333333333333333"}, verification: {verified: true}}'
        ;;
    "POST:repos/$FAKE_TARGET_REPOSITORY/git/blobs")
        encoded="$(jq -er '.content' <<<"$payload")"
        sha="$(printf '%s' "$encoded" | openssl base64 -d -A | git hash-object --stdin)"
        jq -cn --arg sha "$sha" '{sha: $sha}'
        ;;
    "POST:repos/$FAKE_TARGET_REPOSITORY/git/trees")
        jq -e '.tree | length > 0' >/dev/null <<<"$payload"
        jq -cn --arg sha "$FAKE_SOURCE_TREE" '{sha: $sha}'
        ;;
    "POST:repos/$FAKE_TARGET_REPOSITORY/git/commits")
        jq -e \
            --arg message "release: VifuGodot $FAKE_RELEASE_TAG from Vifu $FAKE_SOURCE_COMMIT" \
            --arg tree "$FAKE_SOURCE_TREE" \
            --arg parent "$FAKE_PARENT_COMMIT" \
            '.message == $message and .tree == $tree and .parents == [$parent] and (has("author") | not) and (has("committer") | not) and (has("signature") | not)' \
            >/dev/null <<<"$payload"
        printf 'created\n' >> "$FAKE_STATE_DIR/commits"
        jq -cn \
            --arg sha "$FAKE_RELEASE_COMMIT" \
            --arg tree "$FAKE_SOURCE_TREE" \
            '{sha: $sha, tree: {sha: $tree}, verification: {verified: true, reason: "valid"}}'
        ;;
    "PATCH:repos/$FAKE_TARGET_REPOSITORY/git/refs/heads/main")
        [ "$(field_value sha)" = "$FAKE_RELEASE_COMMIT" ]
        [ "$(field_value force)" = "false" ]
        printf 'updated\n' > "$FAKE_STATE_DIR/main-updated"
        jq -cn --arg sha "$FAKE_RELEASE_COMMIT" \
            '{ref: "refs/heads/main", object: {type: "commit", sha: $sha}}'
        ;;
    "POST:repos/$FAKE_TARGET_REPOSITORY/git/refs")
        [ "$(field_value ref)" = "refs/tags/$FAKE_RELEASE_TAG" ]
        [ "$(field_value sha)" = "$FAKE_RELEASE_COMMIT" ]
        printf 'created\n' > "$FAKE_STATE_DIR/tag-created"
        jq -cn --arg sha "$FAKE_RELEASE_COMMIT" \
            --arg tag "$FAKE_RELEASE_TAG" \
            '{ref: ("refs/tags/" + $tag), object: {type: "commit", sha: $sha}}'
        ;;
    "GET:repos/$FAKE_TARGET_REPOSITORY/commits/$FAKE_RELEASE_TAG")
        [ -f "$FAKE_STATE_DIR/tag-created" ]
        jq -cn \
            --arg sha "$FAKE_RELEASE_COMMIT" \
            --arg tree "$FAKE_SOURCE_TREE" \
            '{sha: $sha, commit: {tree: {sha: $tree}, verification: {verified: true, reason: "valid"}}}'
        ;;
    *)
        echo "Unexpected fake gh api request: $method $endpoint" >&2
        exit 2
        ;;
esac
EOF
chmod +x "$FAKE_BIN/gh"

export FAKE_PARENT_COMMIT="$PARENT_COMMIT"
export FAKE_RELEASE_COMMIT="$RELEASE_COMMIT"
export FAKE_SOURCE_COMMIT="$SOURCE_COMMIT"
export FAKE_SOURCE_TREE="$SOURCE_TREE"
export FAKE_STATE_DIR="$STATE_DIR"
export FAKE_TARGET_REPOSITORY="$TARGET_REPOSITORY"
export FAKE_RELEASE_TAG="$RELEASE_TAG"

(
    cd "$SOURCE_DIR"
    PATH="$FAKE_BIN:$PATH" \
    GH_TOKEN=fake-installation-token \
    VIFUGODOT_RELEASE_REPOSITORY="$TARGET_REPOSITORY" \
        bash scripts/publish-vifugodot.sh "$RELEASE_TAG"
)

test -f "$STATE_DIR/main-updated"
test -f "$STATE_DIR/tag-created"
test -f "$STATE_DIR/release-created"
test "$(wc -l < "$STATE_DIR/commits" | tr -d ' ')" = "1"

# A completed release is idempotent and must not create a second snapshot.
(
    cd "$SOURCE_DIR"
    PATH="$FAKE_BIN:$PATH" \
    GH_TOKEN=fake-installation-token \
    VIFUGODOT_RELEASE_REPOSITORY="$TARGET_REPOSITORY" \
        bash scripts/publish-vifugodot.sh "$RELEASE_TAG"
)
test "$(wc -l < "$STATE_DIR/commits" | tr -d ' ')" = "1"

echo "VifuGodot publish tool tests passed"
