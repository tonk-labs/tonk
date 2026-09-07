#!/bin/sh
# Stamp service_worker.js with the identity of its final browser artifact set.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: stamp-service-worker.sh <dist-dir>" >&2
    exit 2
fi

DIST=$1
SW="$DIST/service_worker.js"
WORKER_WASM="$DIST/worker_bg.wasm"
WORKER_GLUE="$DIST/worker.js"
INDEX="$DIST/index.html"
VERSION="$DIST/version.json"
MANIFEST="$DIST/asset-manifest.json"
LOCK="$DIST/.tonk-stamp.lock"

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
[ -f "$INDEX" ] || {
    echo "stamp-service-worker: missing $INDEX" >&2
    exit 1
}

hash_stream_full() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -c1-64
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | cut -c1-64
    else
        echo "stamp-service-worker: sha256sum or shasum is required" >&2
        exit 1
    fi
}

hash_stream() {
    hash_stream_full | cut -c1-16
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

hash_file_full() {
    HASH=$(hash_stream_full < "$1")
    if [ "${#HASH}" -ne 64 ]; then
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

if ! mkdir "$LOCK" 2>/dev/null; then
    echo "stamp-service-worker: another stamp is publishing $DIST" >&2
    exit 1
fi

SW_TMP="$SW.tmp.$$"
INDEX_TMP="$INDEX.tmp.$$"
VERSION_TMP="$VERSION.tmp.$$"
MANIFEST_TMP="$MANIFEST.tmp.$$"
SW_BACKUP="$SW.stamp-backup.$$"
INDEX_BACKUP="$INDEX.stamp-backup.$$"
VERSION_BACKUP="$VERSION.stamp-backup.$$"
MANIFEST_BACKUP="$MANIFEST.stamp-backup.$$"
ASSET_LIST_UNSORTED="$LOCK/assets.unsorted"
ASSET_LIST="$LOCK/assets"
ASSET_GRAPH="$LOCK/graph"
ASSET_FILES_UNSORTED="$LOCK/files.unsorted"
ASSET_FILES="$LOCK/files"
NORMALIZED_INDEX="$LOCK/index.normalized"
NORMALIZED_SW="$LOCK/service-worker.normalized"
BUILD_INPUT="$LOCK/build-input"
BACKED_UP=0
COMMITTED=0
VERSION_EXISTED=0
MANIFEST_EXISTED=0

cleanup() {
    STATUS=$?
    trap - EXIT HUP INT TERM
    # A failed cleanup must not abort halfway under `set -e`: make every
    # restore attempt, retain backups if any restore fails, and leave the lock
    # behind so a later publisher cannot overwrite the forensic recovery set.
    set +e
    RESTORE_FAILED=0
    if [ "$BACKED_UP" -eq 1 ] && [ "$COMMITTED" -ne 1 ]; then
        mv -f "$SW_BACKUP" "$SW" || RESTORE_FAILED=1
        mv -f "$INDEX_BACKUP" "$INDEX" || RESTORE_FAILED=1
        if [ "$VERSION_EXISTED" -eq 1 ]; then
            mv -f "$VERSION_BACKUP" "$VERSION" || RESTORE_FAILED=1
        else
            rm -f "$VERSION" || RESTORE_FAILED=1
        fi
        if [ "$MANIFEST_EXISTED" -eq 1 ]; then
            mv -f "$MANIFEST_BACKUP" "$MANIFEST" || RESTORE_FAILED=1
        else
            rm -f "$MANIFEST" || RESTORE_FAILED=1
        fi
    fi
    rm -f "$SW_TMP" "$INDEX_TMP" "$VERSION_TMP" "$MANIFEST_TMP"
    rm -f "$ASSET_LIST_UNSORTED" "$ASSET_LIST" "$ASSET_GRAPH"
    rm -f "$ASSET_FILES_UNSORTED" "$ASSET_FILES"
    rm -f "$NORMALIZED_INDEX" "$NORMALIZED_SW" "$BUILD_INPUT"
    if [ "$RESTORE_FAILED" -eq 0 ]; then
        rm -f "$SW_BACKUP" "$INDEX_BACKUP" "$VERSION_BACKUP" "$MANIFEST_BACKUP"
        rmdir "$LOCK" 2>/dev/null
    else
        STATUS=1
        echo "stamp-service-worker: rollback failed; backups and lock retained in $DIST" >&2
    fi
    exit "$STATUS"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

grep -q '^const BUILD_ID = ' "$SW" || {
    echo "stamp-service-worker: $SW has no BUILD_ID declaration" >&2
    exit 1
}
grep -q '^const WORKER_WASM_HASH = ' "$SW" || {
    echo "stamp-service-worker: $SW has no WORKER_WASM_HASH declaration" >&2
    exit 1
}
grep -q '^const ASSET_MANIFEST_HASH = ' "$SW" || {
    echo "stamp-service-worker: $SW has no ASSET_MANIFEST_HASH declaration" >&2
    exit 1
}
grep -q '^const ASSET_PATHS = ' "$SW" || {
    echo "stamp-service-worker: $SW has no ASSET_PATHS declaration" >&2
    exit 1
}
if [ "$(grep -c '<meta name="tonk-worker-build" content="' "$INDEX")" -ne 1 ]; then
    echo "stamp-service-worker: $INDEX must have one tonk-worker-build meta tag" >&2
    exit 1
fi

# The publisher owns the resource-graph interface. Enumerate every browser
# resource in the completed dist, excluding service-worker-owned scripts and
# live deployment metadata. The root document is canonicalized before hashing
# so its generated build meta tag does not make BUILD_ID self-referential.
if ! find "$DIST" -type f > "$ASSET_FILES_UNSORTED"; then
    echo "stamp-service-worker: failed to enumerate $DIST" >&2
    exit 1
fi
LC_ALL=C sort "$ASSET_FILES_UNSORTED" > "$ASSET_FILES"
while IFS= read -r FILE; do
    REL=${FILE#"$DIST"/}
    case "$REL" in
        service_worker.js | worker.js | worker_bg.wasm | version.json | asset-manifest.json | _headers | \
            .tonk-stamp.lock | .tonk-stamp.lock/* | *.tmp.* | *.stamp-backup.*)
            continue
            ;;
    esac
    case "$REL" in
        *'"'* | *'\\'* | *'|'*)
            echo "stamp-service-worker: unsupported asset path '$REL'" >&2
            exit 1
            ;;
    esac
    if [ "$REL" = "index.html" ]; then
        ROUTE=/
        printf '%s|%s\n' "$ROUTE" "$REL"
    elif [ "${REL%/index.html}" != "$REL" ]; then
        # Static sites are navigated by their directory URL. Stamp both the
        # physical member and the explicit directory alias so offline routing
        # never guesses that an arbitrary app path is an immutable document.
        ROUTE=/$REL
        printf '%s|%s\n' "$ROUTE" "$REL"
        printf '%s|%s\n' "/${REL%index.html}" "$REL"
    else
        ROUTE=/$REL
        printf '%s|%s\n' "$ROUTE" "$REL"
    fi
done < "$ASSET_FILES" > "$ASSET_LIST_UNSORTED"
LC_ALL=C sort "$ASSET_LIST_UNSORTED" > "$ASSET_LIST"
grep -q '^/|index.html$' "$ASSET_LIST" || {
    echo "stamp-service-worker: asset graph has no root document" >&2
    exit 1
}

while IFS='|' read -r ROUTE REL; do
    if [ "$REL" = "index.html" ]; then
        sed '/<meta name="tonk-worker-build" content="/ s/content="[^"]*"/content="dev"/' \
            "$DIST/$REL" > "$NORMALIZED_INDEX"
        ASSET_HASH=$(hash_file_full "$NORMALIZED_INDEX")
    else
        ASSET_HASH=$(hash_file_full "$DIST/$REL")
    fi
    printf '%s|%s\n' "$ROUTE" "$ASSET_HASH"
done < "$ASSET_LIST" > "$ASSET_GRAPH"

WASM_HASH=$(hash_file "$WORKER_WASM")
GLUE_HASH=$(hash_file "$WORKER_GLUE")
ASSET_GRAPH_HASH=$(hash_file "$ASSET_GRAPH")
ASSET_PATHS_DECL='const ASSET_PATHS = ['
FIRST=1
while IFS='|' read -r ROUTE REL; do
    if [ "$FIRST" -eq 1 ]; then
        FIRST=0
    else
        ASSET_PATHS_DECL="$ASSET_PATHS_DECL,"
    fi
    ASSET_PATHS_DECL="$ASSET_PATHS_DECL\"$ROUTE\""
done < "$ASSET_LIST"
ASSET_PATHS_DECL="$ASSET_PATHS_DECL];"
ASSET_PATHS_SED=$(printf '%s' "$ASSET_PATHS_DECL" | sed 's/[&]/\\&/g')
# Include the outer service-worker policy without hashing generated identities
# back into itself. Any policy, worker, or published resource change therefore
# produces a new immutable generation while restamping stays deterministic.
sed -e 's|^const BUILD_ID = .*|const BUILD_ID = "dev";|' \
    -e 's|^const WORKER_WASM_HASH = .*|const WORKER_WASM_HASH = "dev";|' \
    -e 's|^const ASSET_MANIFEST_HASH = .*|const ASSET_MANIFEST_HASH = "dev";|' \
    -e 's|^const ASSET_PATHS = .*|const ASSET_PATHS = ["dev"];|' \
    "$SW" > "$NORMALIZED_SW"
SW_HASH=$(hash_file "$NORMALIZED_SW")
printf '%s\n' "$SW_HASH" "$WASM_HASH" "$GLUE_HASH" "$ASSET_GRAPH_HASH" > "$BUILD_INPUT"
BUILD_ID=$(hash_file "$BUILD_INPUT")
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

sed '/<meta name="tonk-worker-build" content="/ s/content="[^"]*"/content="'"$BUILD_ID"'"/' \
    "$INDEX" > "$INDEX_TMP"
grep -q "name=\"tonk-worker-build\" content=\"$BUILD_ID\"" "$INDEX_TMP" || {
    echo "stamp-service-worker: document build verification failed" >&2
    exit 1
}

{
    printf '{\n  "version": 1,\n  "build": "%s",\n  "assets": {' "$BUILD_ID"
    FIRST=1
    while IFS='|' read -r ROUTE REL; do
        if [ "$REL" = "index.html" ]; then
            ASSET_HASH=$(hash_file_full "$INDEX_TMP")
        else
            ASSET_HASH=$(hash_file_full "$DIST/$REL")
        fi
        if [ "$FIRST" -eq 1 ]; then
            FIRST=0
        else
            printf ','
        fi
        printf '\n    "%s": "%s"' "$ROUTE" "$ASSET_HASH"
    done < "$ASSET_LIST"
    printf '\n  }\n}\n'
} > "$MANIFEST_TMP"
MANIFEST_HASH=$(hash_file_full "$MANIFEST_TMP")

sed -e "s|^const BUILD_ID = .*|const BUILD_ID = \"$BUILD_ID\";|" \
    -e "s|^const WORKER_WASM_HASH = .*|const WORKER_WASM_HASH = \"$WASM_HASH\";|" \
    -e "s|^const ASSET_MANIFEST_HASH = .*|const ASSET_MANIFEST_HASH = \"$MANIFEST_HASH\";|" \
    -e "s|^const ASSET_PATHS = .*|$ASSET_PATHS_SED|" \
    "$SW" > "$SW_TMP"
grep -q "^const BUILD_ID = \"$BUILD_ID\";$" "$SW_TMP" || {
    echo "stamp-service-worker: BUILD_ID verification failed" >&2
    exit 1
}
grep -q "^const WORKER_WASM_HASH = \"$WASM_HASH\";$" "$SW_TMP" || {
    echo "stamp-service-worker: WORKER_WASM_HASH verification failed" >&2
    exit 1
}
grep -q "^const ASSET_MANIFEST_HASH = \"$MANIFEST_HASH\";$" "$SW_TMP" || {
    echo "stamp-service-worker: ASSET_MANIFEST_HASH verification failed" >&2
    exit 1
}
grep -Fqx "$ASSET_PATHS_DECL" "$SW_TMP" || {
    echo "stamp-service-worker: ASSET_PATHS verification failed" >&2
    exit 1
}

printf '{ "build": "%s", "serviceWorker": "%s", "workerWasm": "%s", "assetManifest": "%s" }\n' \
    "$BUILD_ID" "$SW_HASH" "$WASM_HASH" "$MANIFEST_HASH" > "$VERSION_TMP"

# All outputs are complete and validated before publication. POSIX cannot
# rename four files as one operation, so retain originals and roll back every
# catchable failure; the lock prevents two stampers interleaving their moves.
cp "$SW" "$SW_BACKUP"
cp "$INDEX" "$INDEX_BACKUP"
if [ -f "$VERSION" ]; then
    cp "$VERSION" "$VERSION_BACKUP"
    VERSION_EXISTED=1
fi
if [ -f "$MANIFEST" ]; then
    cp "$MANIFEST" "$MANIFEST_BACKUP"
    MANIFEST_EXISTED=1
fi
BACKED_UP=1
mv -f "$SW_TMP" "$SW"
mv -f "$INDEX_TMP" "$INDEX"
mv -f "$MANIFEST_TMP" "$MANIFEST"
mv -f "$VERSION_TMP" "$VERSION"

grep -q "^const BUILD_ID = \"$BUILD_ID\";$" "$SW"
grep -q "name=\"tonk-worker-build\" content=\"$BUILD_ID\"" "$INDEX"
grep -q "\"build\": \"$BUILD_ID\"" "$MANIFEST"
grep -q "\"build\": \"$BUILD_ID\"" "$VERSION"
COMMITTED=1

echo "stamp-service-worker: build=$BUILD_ID sw=$SW_HASH wasm=$WASM_HASH manifest=$MANIFEST_HASH"
