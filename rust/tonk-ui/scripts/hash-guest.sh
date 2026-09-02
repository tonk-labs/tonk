#!/bin/sh
# Content-hash the sealed-guest runtime assets so they cache forever.
#
# Trunk's `rel="rust" data-type="worker"` builds the guest bundle but emits
# fixed names (`guest.js`, `guest_bg.wasm`) — workers are named at runtime,
# so trunk deliberately skips hashing them. The esbuild-produced `wa.js` /
# `wa.css` ride along via copy-dir and aren't hashed either. Unhashed names
# on a stable URL are stale-cache bait unless their exact bytes are bound into
# the generation that installs them.
#
# This post_build hook (TRUNK_STAGING_DIR is the staged dist before serving)
# renames each guest asset to `<name>-<hash>.<ext>` and writes a tiny
# `guest/manifest.json` mapping logical -> hashed name. The portal fetches
# the manifest, then loads the hashed assets. The outer build publisher records
# the manifest and all of those files in its full-digest resource graph, so a
# generation installs them together and serves them immutably offline.
#
# The guest's own `snippets/*` are imported by RELATIVE path inside
# guest.js and fetched as-is by the portal, so they stay unhashed (their
# content rarely changes and they're small); only the four top-level assets
# are hashed.
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

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

"$SCRIPT_DIR/stamp-service-worker.sh" "${TRUNK_STAGING_DIR:?TRUNK_STAGING_DIR not set}"
