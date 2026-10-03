#!/bin/sh
# Build the current worktree's opt-in browser artifact without staging files.
set -eu
if [ "$#" -ne 1 ]; then
    echo "usage: build-connection-test.sh <artifact-directory>" >&2
    exit 2
fi
UI_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$1"
ARTIFACT=$(CDPATH= cd -- "$1" && pwd)
cd "$UI_ROOT"
HTML=$(mktemp "$UI_ROOT/.connection-test.XXXXXX.html")
trap 'rm -f "$HTML"' EXIT HUP INT TERM
# Trunk's global --features also affects tonk-guest, which does not expose this
# worker feature. Select it only on the two tonk-ui binary build pipelines.
sed -e 's/data-bin="ui"/data-bin="ui" data-cargo-features="connection-invites"/' \
    -e 's/data-bin="worker"/data-bin="worker" data-cargo-features="connection-invites"/' \
    index.html > "$HTML"
env -u NO_COLOR trunk build "$HTML" --html-output index.html \
    --dist "$ARTIFACT" --offline --release --color never
