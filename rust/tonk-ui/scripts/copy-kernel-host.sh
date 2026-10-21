#!/bin/sh
# Serve what spaces that run a kernel need, under `/_host/`: the embedder a
# space's bootloader loads, and the kernel this deployment installs into a
# space it creates (see `plan/space-kernel.md`). Built apart, by
# `kernel/build-host.sh`, since the kernel builds for `wasm32-wasip3` on a
# toolchain of its own. A build without it creates spaces as before.
set -eu
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
HOST_DIR="$SCRIPT_DIR/../../../kernel/dist/_host"
[ -f "$HOST_DIR/kernel.json" ] || { echo "copy-kernel-host: no $HOST_DIR, skipping"; exit 0; }
rm -rf "${TRUNK_STAGING_DIR:?TRUNK_STAGING_DIR not set}/_host"
cp -R "$HOST_DIR" "$TRUNK_STAGING_DIR/_host"
