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

# Short content hash of a file (first 16 hex chars of sha256).
hash_of() { shasum -a 256 "$1" | cut -c1-16; }

# Rename "$1" (a file in $GUEST_DIR) to "<stem>-<hash>.<ext>" and echo the
# hashed basename. Splits on the FIRST dot so `guest_bg.wasm` -> stem
# `guest_bg`, ext `wasm`.
hash_rename() {
    src="$GUEST_DIR/$1"
    [ -f "$src" ] || { echo ""; return 0; }
    h=$(hash_of "$src")
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

cat > "$GUEST_DIR/manifest.json" <<EOF
{
  "js": "$JS",
  "wasm": "$WASM",
  "waJs": "$WA_JS",
  "waCss": "$WA_CSS"
}
EOF

echo "hash-guest: js=$JS wasm=$WASM waJs=$WA_JS waCss=$WA_CSS"
