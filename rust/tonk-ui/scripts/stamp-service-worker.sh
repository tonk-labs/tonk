#!/bin/sh
# Stamp service_worker.js with the identity of its final worker artifact set.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: stamp-service-worker.sh <dist-dir>" >&2
    exit 2
fi

DIST=$1
SW="$DIST/service_worker.js"
WORKER_WASM="$DIST/worker_bg.wasm"
WORKER_GLUE="$DIST/worker.js"
VERSION="$DIST/version.json"

[ -f "$SW" ] || {
    echo "stamp-service-worker: missing $SW" >&2
    exit 1
}
[ -f "$WORKER_WASM" ] || {
    echo "stamp-service-worker: missing $WORKER_WASM" >&2
    exit 1
}
[ -f "$WORKER_GLUE" ] || {
    echo "stamp-service-worker: missing $WORKER_GLUE" >&2
    exit 1
}

hash_stream() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -c1-16
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | cut -c1-16
    else
        echo "stamp-service-worker: sha256sum or shasum is required" >&2
        exit 1
    fi
}

hash_file() {
    HASH=$(hash_stream < "$1")
    if [ "${#HASH}" -ne 16 ]; then
        echo "stamp-service-worker: malformed hash '$HASH' for $1" >&2
        exit 1
    fi
    case "$HASH" in
        *[!0-9a-f]*)
            echo "stamp-service-worker: malformed hash '$HASH' for $1" >&2
            exit 1
            ;;
    esac
    printf '%s\n' "$HASH"
}

WASM_HASH=$(hash_file "$WORKER_WASM")
GLUE_HASH=$(hash_file "$WORKER_GLUE")
BUILD_ID=$(printf '%s\n' "$WASM_HASH" "$GLUE_HASH" | hash_stream)
if [ "${#BUILD_ID}" -ne 16 ]; then
    echo "stamp-service-worker: malformed build id '$BUILD_ID'" >&2
    exit 1
fi
case "$BUILD_ID" in
    *[!0-9a-f]*)
        echo "stamp-service-worker: malformed build id '$BUILD_ID'" >&2
        exit 1
        ;;
esac

TMP="$SW.tmp"
VERSION_TMP="$VERSION.tmp"
trap 'rm -f "$TMP" "$VERSION_TMP"' EXIT HUP INT TERM

grep -q '^const BUILD_ID = ' "$SW" || {
    echo "stamp-service-worker: $SW has no BUILD_ID declaration" >&2
    exit 1
}
grep -q '^const WORKER_WASM_HASH = ' "$SW" || {
    echo "stamp-service-worker: $SW has no WORKER_WASM_HASH declaration" >&2
    exit 1
}

sed -e "s|^const BUILD_ID = .*|const BUILD_ID = \"$BUILD_ID\";|" \
    -e "s|^const WORKER_WASM_HASH = .*|const WORKER_WASM_HASH = \"$WASM_HASH\";|" \
    "$SW" > "$TMP"
grep -q "^const BUILD_ID = \"$BUILD_ID\";$" "$TMP" || {
    echo "stamp-service-worker: BUILD_ID verification failed" >&2
    exit 1
}
grep -q "^const WORKER_WASM_HASH = \"$WASM_HASH\";$" "$TMP" || {
    echo "stamp-service-worker: WORKER_WASM_HASH verification failed" >&2
    exit 1
}

printf '{ "build": "%s", "workerWasm": "%s" }\n' \
    "$BUILD_ID" "$WASM_HASH" > "$VERSION_TMP"

mv -f "$TMP" "$SW"
mv -f "$VERSION_TMP" "$VERSION"
trap - EXIT HUP INT TERM

echo "stamp-service-worker: build=$BUILD_ID wasm=$WASM_HASH"
