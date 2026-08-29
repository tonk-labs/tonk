#!/bin/sh
# Content-hash the sealed-guest runtime assets so they cache forever.
#
# Trunk's `rel="rust" data-type="worker"` builds the guest bundle but emits
# fixed names (`guest.js`, `guest_bg.wasm`) — workers are named at runtime,
# so trunk deliberately skips hashing them. The esbuild-produced `wa.js` /
# `wa.css` ride along via copy-dir and aren't hashed either. Unhashed names
# on a stable URL are stale-cache bait: the SW serves the previous build
# until stale-while-revalidate catches up a load later.
#
# This post_build hook (TRUNK_STAGING_DIR is the staged dist before serving)
# renames each guest asset to `<name>-<hash>.<ext>` and writes a tiny
# `guest/manifest.json` mapping logical -> hashed name. The portal fetches
# the manifest (cache: no-store, it's ~120 bytes) then loads the hashed
# assets, which can be cached immutably. A content change => new hash => new
# URL => cache miss => fresh, with no one-load-behind lag.
#
# The guest's own `snippets/*` are imported by RELATIVE path inside
# guest.js and fetched as-is by the portal, so they stay unhashed (their
# content rarely changes and they're small); only the four top-level assets
# are hashed.
set -eu

GUEST_DIR="${TRUNK_STAGING_DIR:?TRUNK_STAGING_DIR not set}/guest"
[ -d "$GUEST_DIR" ] || { echo "hash-guest: no $GUEST_DIR, skipping"; exit 0; }

# Short content hash of a file (first 16 hex chars of sha256). Prefer
# sha256sum (coreutils — present in the Nix build sandbox, where perl's
# shasum is not); fall back to shasum for stock macOS.
hash_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -c1-16
    else
        shasum -a 256 "$1" | cut -c1-16
    fi
}

# Rename "$1" (a file in $GUEST_DIR) to "<stem>-<hash>.<ext>" and echo the
# hashed basename. Splits on the FIRST dot so `guest_bg.wasm` -> stem
# `guest_bg`, ext `wasm`.
hash_rename() {
    src="$GUEST_DIR/$1"
    [ -f "$src" ] || { echo ""; return 0; }
    h=$(hash_of "$src")
    # An empty or malformed hash means the hashing tool is missing or broke,
    # and its exit status was masked by the `| cut` pipeline. Renaming with an
    # empty hash ("guest_bg-.wasm") silently defeats cache busting — this
    # shipped to staging once — so fail the build instead.
    echo "$h" | grep -qE '^[0-9a-f]{16}$' || {
        echo "hash-guest: bad hash '$h' for $1 (sha256sum/shasum missing?)" >&2
        exit 1
    }
    stem=${1%%.*}
    ext=${1#*.}
    hashed="${stem}-${h}.${ext}"
    mv -f "$src" "$GUEST_DIR/$hashed"
    echo "$hashed"
}

JS=$(hash_rename "guest.js")
WASM=$(hash_rename "guest_bg.wasm")
WA_JS=$(hash_rename "wa.js")
WA_CSS=$(hash_rename "wa.css")

# The wasm-bindgen `.d.ts` files are dev-only type stubs; drop them from the
# served dist (the portal never fetches them).
rm -f "$GUEST_DIR"/guest.d.ts "$GUEST_DIR"/guest_bg.wasm.d.ts

# All four assets are required by the portal at runtime (bridge.rs fetches
# each one by its manifest name); a missing file here is a broken build, not
# a variant to tolerate. Fail before writing a manifest with empty entries.
for entry in "js=$JS" "wasm=$WASM" "waJs=$WA_JS" "waCss=$WA_CSS"; do
    case "$entry" in
        *=) echo "hash-guest: missing guest asset (${entry%=})" >&2; exit 1 ;;
    esac
done

cat > "$GUEST_DIR/manifest.json" <<EOF
{
  "js": "$JS",
  "wasm": "$WASM",
  "waJs": "$WA_JS",
  "waCss": "$WA_CSS"
}
EOF

echo "hash-guest: js=$JS wasm=$WASM waJs=$WA_JS waCss=$WA_CSS"

# ---------------------------------------------------------------------------
# Service-worker build identity.
#
# The SW script chain — `service_worker.js` (a copied static file) imports
# `worker.js` (trunk's `data-bin=worker` glue) which loads `worker_bg.wasm`
# by a FIXED name — carries no content-dependent identifier anywhere. On an
# update check (`updateViaCache: "none"`, so always network) the browser
# byte-compares the SW script and its imported `worker.js`; both are
# identical across rebuilds even when the wasm changed, so the browser never
# detects an update and the old worker is never replaced.
#
# So stamp a build id into `service_worker.js`. Two properties matter:
#
#  1. It hashes the WHOLE worker artifact set (glue + wasm), not just the
#     wasm. A glue-only change would otherwise rely on the browser
#     byte-checking imported scripts — correct on current engines, but
#     historically unreliable on WebKit, which is the engine that strands
#     users on old workers.
#
#  2. It is a real `const`, not a comment. The shim needs the value at
#     runtime: it names the per-version caches (so two builds never share
#     one cache) and it verifies the wasm the worker precaches at install,
#     which is what keeps glue and wasm from drifting apart.
DIST="${TRUNK_STAGING_DIR:?TRUNK_STAGING_DIR not set}"
SW="$DIST/service_worker.js"
WORKER_WASM="$DIST/worker_bg.wasm"
WORKER_GLUE="$DIST/worker.js"
if [ -f "$SW" ] && [ -f "$WORKER_WASM" ] && [ -f "$WORKER_GLUE" ]; then
    WASM_HASH=$(hash_of "$WORKER_WASM")
    echo "$WASM_HASH" | grep -qE '^[0-9a-f]{16}$' || {
        echo "hash-guest: bad worker wasm hash '$WASM_HASH'" >&2
        exit 1
    }
    # Build id covers glue and wasm together. Hash of the concatenated
    # per-file hashes, so it changes if EITHER half changes.
    BUILD_ID=$(printf '%s\n' "$WASM_HASH" "$(hash_of "$WORKER_GLUE")" \
        | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; } \
        | cut -c1-16)
    echo "$BUILD_ID" | grep -qE '^[0-9a-f]{16}$' || {
        echo "hash-guest: bad worker build id '$BUILD_ID'" >&2
        exit 1
    }
    # Replace the placeholder declarations the checked-in source carries.
    # `__TONK_BUILD_ID__` / `__TONK_WORKER_WASM_HASH__` are the dev-build
    # values; a release build rewrites them in place. Fail loudly if the
    # placeholders are gone — a silent no-op here reintroduces exactly the
    # staleness this hook exists to prevent.
    grep -q 'const BUILD_ID = ' "$SW" || {
        echo "hash-guest: service_worker.js has no BUILD_ID declaration to stamp" >&2
        exit 1
    }
    sed -e "s|^const BUILD_ID = .*|const BUILD_ID = \"$BUILD_ID\";|" \
        -e "s|^const WORKER_WASM_HASH = .*|const WORKER_WASM_HASH = \"$WASM_HASH\";|" \
        "$SW" > "$SW.tmp"
    mv -f "$SW.tmp" "$SW"

    # `version.json` — the page-side update probe (finding 4). Served
    # `no-store`, so it answers correctly even when the SW's own update
    # machinery is wedged, which is the Safari failure mode.
    cat > "$DIST/version.json" <<EOF
{ "build": "$BUILD_ID", "workerWasm": "$WASM_HASH" }
EOF
    echo "hash-guest: stamped service_worker.js build=$BUILD_ID wasm=$WASM_HASH"
else
    echo "hash-guest: no service_worker.js / worker.js / worker_bg.wasm to stamp, skipping"
fi
