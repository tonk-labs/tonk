#!/usr/bin/env bash
# Build the run's hermetic npm registry: stage the bench wrapper
# package with the vendored release binary, npm-pack it, and write a
# static packument so `npx tonk` resolves against
# $BENCH_URL/registry/ with no real-registry traffic.
#
# Env: ROOT, RUN_DIR, BENCH_URL
set -euo pipefail

ROOT="${ROOT:?}"; RUN_DIR="${RUN_DIR:?}"; BENCH_URL="${BENCH_URL:?}"
VERSION="0.0.0-bench"
STAGE="$RUN_DIR/npm-pkg"
REG="$RUN_DIR/registry"

build() {
  [ -x "$ROOT/target/release/tonk" ] || { echo "registry: no release tonk (build with cargo build --release -p tonk-cli)" >&2; exit 1; }
  rm -rf "$STAGE" "$REG"
  mkdir -p "$STAGE" "$REG"
  cp -R "$ROOT/bench/npm/tonk-wrapper/." "$STAGE/"
  mkdir -p "$STAGE/vendor"
  cp "$ROOT/target/release/tonk" "$STAGE/vendor/tonk"
  chmod +x "$STAGE/vendor/tonk" "$STAGE/bin/tonk.js"

  (cd "$STAGE" && npm pack --pack-destination "$REG" >/dev/null)
  TGZ="$REG/tonk-$VERSION.tgz"
  [ -f "$TGZ" ] || { echo "registry: npm pack produced no $TGZ" >&2; exit 1; }

  local sha1 sha512
  sha1="$(shasum -a 1 "$TGZ" | awk '{print $1}')"
  sha512="sha512-$(openssl dgst -sha512 -binary "$TGZ" | base64 | tr -d '\n')"

  jq -n \
    --arg v "$VERSION" \
    --arg tarball "$BENCH_URL/registry/tonk-$VERSION.tgz" \
    --arg sha1 "$sha1" --arg sha512 "$sha512" '
  {
    name: "tonk",
    "dist-tags": { latest: $v },
    versions: {
      ($v): {
        name: "tonk", version: $v,
        bin: { tonk: "bin/tonk.js" },
        dist: { tarball: $tarball, shasum: $sha1, integrity: $sha512 }
      }
    }
  }' > "$REG/tonk.json"
  echo "registry: built at $REG (serve as $BENCH_URL/registry/)" >&2
}

case "${1:-}" in
  build) build ;;
  *) echo "usage: registry.sh build" >&2; exit 2 ;;
esac
