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

# The content identifier of a file: a CIDv1 with the raw codec and the
# file's SHA-256, in base32. It is what names a release.
cid_of_file() {
    printf '01551220%s' "$(hash_file_full "$1")" | awk '
        BEGIN { hex = "0123456789abcdef"; alphabet = "abcdefghijklmnopqrstuvwxyz234567" }
        {
            bits = 0; held = 0; out = "b"
            for (i = 1; i <= length($0); i++) {
                held = held * 16 + index(hex, substr($0, i, 1)) - 1
                bits += 4
                while (bits >= 5) {
                    bits -= 5
                    out = out substr(alphabet, int(held / 2 ^ bits) + 1, 1)
                    held = held % 2 ^ bits
                }
            }
            if (bits > 0) out = out substr(alphabet, held * 2 ^ (5 - bits) + 1, 1)
            print out
        }'
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
PAGE_GRAPH="$LOCK/page-graph"
ASSET_FILES_UNSORTED="$LOCK/files.unsorted"
ASSET_FILES="$LOCK/files"
NORMALIZED_INDEX="$LOCK/index.normalized"
CANONICAL_INDEX="$LOCK/index.canonical"
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
    rm -f "$ASSET_LIST_UNSORTED" "$ASSET_LIST" "$ASSET_GRAPH" "$PAGE_GRAPH"
    rm -f "$ASSET_FILES_UNSORTED" "$ASSET_FILES"
    rm -f "$NORMALIZED_INDEX" "$CANONICAL_INDEX" "$NORMALIZED_SW" "$BUILD_INPUT"
    rm -f "$LOCK/space-worker" "$LOCK/release.js" "$LOCK/boot"
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
if [ "$(grep -c '<meta name="tonk-worker-build" content="' "$INDEX")" -ne 1 ]; then
    echo "stamp-service-worker: $INDEX must have one tonk-worker-build meta tag" >&2
    exit 1
fi
if [ "$(grep -c '<meta name="tonk-page-build" content="' "$INDEX")" -ne 1 ]; then
    echo "stamp-service-worker: $INDEX must have one tonk-page-build meta tag" >&2
    exit 1
fi

# A site origin's release runs the same worker Wasm and pins it by this hash.
# Stamped before the resource graph is hashed, so the build identity covers
# the stamped file; restamping writes the same line.
SPACE_SW="$DIST/release.js"
if [ -f "$SPACE_SW" ]; then
    grep -q '^const WORKER_WASM_HASH = ' "$SPACE_SW" || {
        echo "stamp-service-worker: $SPACE_SW has no WORKER_WASM_HASH declaration" >&2
        exit 1
    }
    SPACE_WASM_HASH=$(hash_file "$WORKER_WASM")
    SPACE_SW_TMP="$LOCK/space-worker"
    sed -e "s|^const WORKER_WASM_HASH = .*|const WORKER_WASM_HASH = \"$SPACE_WASM_HASH\";|" \
        "$SPACE_SW" > "$SPACE_SW_TMP"
    grep -q "^const WORKER_WASM_HASH = \"$SPACE_WASM_HASH\";$" "$SPACE_SW_TMP" || {
        echo "stamp-service-worker: space worker WORKER_WASM_HASH verification failed" >&2
        exit 1
    }
    mv -f "$SPACE_SW_TMP" "$SPACE_SW"
fi

# A site origin's worker is the bootloader, which loads a release: the site
# worker's script bundled with the bindings of the wasm it runs, named by the
# content identifier of that one file, with the wasm beside it. The
# bootloader is stamped with the release built here, so a site that pins none
# runs this one. Bundled from inside the dist, so that what the bundle says
# of where its parts came from does not depend on where the dist is.
BOOT="$DIST/boot.js"
if [ -f "$SPACE_SW" ] && [ -f "$BOOT" ]; then
    grep -q '^const CURRENT_RELEASE = ' "$BOOT" || {
        echo "stamp-service-worker: $BOOT has no CURRENT_RELEASE declaration" >&2
        exit 1
    }
    ESBUILD=${ESBUILD_BIN:-esbuild}
    command -v "$ESBUILD" >/dev/null 2>&1 || {
        echo "stamp-service-worker: esbuild is required to bundle a release" >&2
        exit 1
    }
    RELEASE_TMP="$LOCK/release.js"
    (cd "$DIST" && "$ESBUILD" release.js --bundle --format=iife --log-level=error \
        --outfile=.tonk-stamp.lock/release.js) || {
        echo "stamp-service-worker: bundling the release failed" >&2
        exit 1
    }
    RELEASE=$(cid_of_file "$RELEASE_TMP")
    case "$RELEASE" in
        bafkrei*[!a-z2-7]* | "")
            echo "stamp-service-worker: malformed release '$RELEASE'" >&2
            exit 1
            ;;
        bafkrei*) ;;
        *)
            echo "stamp-service-worker: malformed release '$RELEASE'" >&2
            exit 1
            ;;
    esac
    if [ "${#RELEASE}" -ne 59 ]; then
        echo "stamp-service-worker: malformed release '$RELEASE'" >&2
        exit 1
    fi
    # Published whole or not at all: a release is immutable, and one that
    # is there already under this name is this one.
    rm -rf "$DIST/_release/$RELEASE"
    mkdir -p "$DIST/_release/$RELEASE"
    mv -f "$RELEASE_TMP" "$DIST/_release/$RELEASE/worker.js"
    cp "$WORKER_WASM" "$DIST/_release/$RELEASE/worker_bg.wasm"

    # A site may pin a release older than the one built here, and a dist is
    # built from nothing each time. Where a store of releases is named, the
    # one built here is kept in it, the store is held to its newest few, and
    # the dist serves every release the store still has.
    if [ -n "${TONK_RELEASE_STORE:-}" ]; then
        KEEP=${TONK_RELEASE_KEEP:-5}
        mkdir -p "$TONK_RELEASE_STORE"
        if [ ! -d "$TONK_RELEASE_STORE/$RELEASE" ]; then
            cp -R "$DIST/_release/$RELEASE" "$TONK_RELEASE_STORE/$RELEASE.tmp.$$"
            mv "$TONK_RELEASE_STORE/$RELEASE.tmp.$$" "$TONK_RELEASE_STORE/$RELEASE"
        fi
        touch "$TONK_RELEASE_STORE/$RELEASE"
        ls -1t "$TONK_RELEASE_STORE" | tail -n +$((KEEP + 1)) | while IFS= read -r OLD; do
            rm -rf "${TONK_RELEASE_STORE:?}/$OLD"
        done
        for KEPT in "$TONK_RELEASE_STORE"/bafkrei*; do
            [ -d "$KEPT" ] || continue
            NAME=${KEPT##*/}
            [ -d "$DIST/_release/$NAME" ] || cp -R "$KEPT" "$DIST/_release/$NAME"
        done
    fi
    BOOT_TMP="$LOCK/boot"
    sed -e "s|^const CURRENT_RELEASE = .*|const CURRENT_RELEASE = \"/_release/$RELEASE/worker.js\";|" \
        "$BOOT" > "$BOOT_TMP"
    grep -q "^const CURRENT_RELEASE = \"/_release/$RELEASE/worker.js\";$" "$BOOT_TMP" || {
        echo "stamp-service-worker: bootloader CURRENT_RELEASE verification failed" >&2
        exit 1
    }
    mv -f "$BOOT_TMP" "$BOOT"
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
        *.map | service_worker.js | worker.js | worker_bg.wasm | version.json | asset-manifest.json | _headers | \
            .tonk-stamp.lock | .tonk-stamp.lock/* | *.tmp.* | *.stamp-backup.*)
            continue
            ;;
        # The vendored icons: thousands of files that change only when the
        # icon set is bumped, each read on its own when something shows it.
        # They are no part of what a page needs to load.
        webawesome/icons/*)
            continue
            ;;
        # A release: what a site's worker loads and no page does. The
        # bootloader is stamped with it, and that is in the graph.
        _release/*)
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

# Trunk emits the root document's `modulepreload` links in a different order
# on every build. Identity hashes a canonical form: one tag per line without
# indentation, preload links sorted after the rest, whose order is kept
# because it is meaningful.
canonical_index() {
    sed -e '/<meta name="tonk-worker-build" content="/ s/content="[^"]*"/content="dev"/' \
        -e '/<meta name="tonk-page-build" content="/ s/content="[^"]*"/content="dev"/' \
        "$1" | awk '{ gsub(/></, ">\n<"); print }' | sed 's/^[[:space:]]*//' > "$CANONICAL_INDEX"
    grep -v 'rel="modulepreload"' "$CANONICAL_INDEX" || true
    grep 'rel="modulepreload"' "$CANONICAL_INDEX" | LC_ALL=C sort || true
}

while IFS='|' read -r ROUTE REL; do
    if [ "$REL" = "index.html" ]; then
        canonical_index "$DIST/$REL" > "$NORMALIZED_INDEX"
        ASSET_HASH=$(hash_file_full "$NORMALIZED_INDEX")
    else
        ASSET_HASH=$(hash_file_full "$DIST/$REL")
    fi
    printf '%s|%s\n' "$ROUTE" "$ASSET_HASH"
done < "$ASSET_LIST" > "$ASSET_GRAPH"

WASM_HASH=$(hash_file "$WORKER_WASM")
GLUE_HASH=$(hash_file "$WORKER_GLUE")
ASSET_GRAPH_HASH=$(hash_file "$ASSET_GRAPH")
# The page identity covers everything the top-level document runs or reads by
# a name it holds, which is every published resource except the sealed-guest
# runtime and editor bundles (the portal fetches those afresh for each guest it
# mounts), library data (seeded by the worker), and what a site's own origin
# runs: its shell and its worker, which is stamped with the hash of the wasm
# it boots. Two builds with the same page identity differ only in worker and
# guest code, so an open document can keep running while its sites load again
# under the new worker.
grep -v -e '^/guest/' -e '^/tonk-code/' -e '^/tonk-prose/' -e '^/tonk-table/' -e '^/library/' \
    -e '^/space_worker\.js|' -e '^/release\.js|' -e '^/boot\.js|' -e '^/space\.html|' -e '^/profile\.html|' \
    "$ASSET_GRAPH" > "$PAGE_GRAPH" || true
PAGE_BUILD=$(hash_file "$PAGE_GRAPH")
# Include the outer service-worker policy without hashing generated identities
# back into itself. Any policy, worker, or published resource change therefore
# produces a new immutable generation while restamping stays deterministic.
sed -e 's|^const BUILD_ID = .*|const BUILD_ID = "dev";|' "$SW" > "$NORMALIZED_SW"
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

sed -e '/<meta name="tonk-worker-build" content="/ s/content="[^"]*"/content="'"$BUILD_ID"'"/' \
    -e '/<meta name="tonk-page-build" content="/ s/content="[^"]*"/content="'"$PAGE_BUILD"'"/' \
    "$INDEX" > "$INDEX_TMP"
grep -q "name=\"tonk-worker-build\" content=\"$BUILD_ID\"" "$INDEX_TMP" || {
    echo "stamp-service-worker: document build verification failed" >&2
    exit 1
}
grep -q "name=\"tonk-page-build\" content=\"$PAGE_BUILD\"" "$INDEX_TMP" || {
    echo "stamp-service-worker: document page build verification failed" >&2
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

sed -e "s|^const BUILD_ID = .*|const BUILD_ID = \"$BUILD_ID\";|" "$SW" > "$SW_TMP"
grep -q "^const BUILD_ID = \"$BUILD_ID\";$" "$SW_TMP" || {
    echo "stamp-service-worker: BUILD_ID verification failed" >&2
    exit 1
}

printf '{ "build": "%s", "page": "%s", "serviceWorker": "%s", "workerWasm": "%s", "assetManifest": "%s" }\n' \
    "$BUILD_ID" "$PAGE_BUILD" "$SW_HASH" "$WASM_HASH" "$MANIFEST_HASH" > "$VERSION_TMP"

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

echo "stamp-service-worker: build=$BUILD_ID page=$PAGE_BUILD sw=$SW_HASH wasm=$WASM_HASH manifest=$MANIFEST_HASH"
