#!/bin/sh
# Stamp service_worker.js with the hash of the final worker_bg.wasm bytes.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: stamp-service-worker.sh <dist-dir>" >&2
    exit 2
fi

DIST=$1
SW="$DIST/service_worker.js"
WORKER_WASM="$DIST/worker_bg.wasm"

[ -f "$SW" ] || {
    echo "stamp-service-worker: missing $SW" >&2
    exit 1
}
[ -f "$WORKER_WASM" ] || {
    echo "stamp-service-worker: missing $WORKER_WASM" >&2
    exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
    HASH=$(sha256sum "$WORKER_WASM" | cut -c1-16)
elif command -v shasum >/dev/null 2>&1; then
    HASH=$(shasum -a 256 "$WORKER_WASM" | cut -c1-16)
else
    echo "stamp-service-worker: sha256sum or shasum is required" >&2
    exit 1
fi

if [ "${#HASH}" -ne 16 ]; then
    echo "stamp-service-worker: malformed worker hash '$HASH'" >&2
    exit 1
fi
case "$HASH" in
    *[!0-9a-f]*)
        echo "stamp-service-worker: malformed worker hash '$HASH'" >&2
        exit 1
        ;;
esac

TMP="$SW.tmp"
trap 'rm -f "$TMP"' EXIT HUP INT TERM
status=0
grep -v '^// worker-wasm-hash:' "$SW" > "$TMP" || status=$?
if [ "$status" -gt 1 ]; then
    echo "stamp-service-worker: failed to remove the old marker" >&2
    exit "$status"
fi
printf '// worker-wasm-hash: %s\n' "$HASH" >> "$TMP"
mv -f "$TMP" "$SW"
trap - EXIT HUP INT TERM

ACTUAL=$(sed -n 's#^// worker-wasm-hash: ##p' "$SW")
COUNT=$(grep -c '^// worker-wasm-hash:' "$SW")
if [ "$ACTUAL" != "$HASH" ] || [ "$COUNT" -ne 1 ]; then
    echo "stamp-service-worker: marker verification failed" >&2
    exit 1
fi

echo "stamp-service-worker: worker-wasm-hash=$HASH"
