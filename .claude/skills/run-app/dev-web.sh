#!/usr/bin/env bash
# Start tonk's dev server without nix: the local access service plus
# `trunk serve`, with the proxies `nix develop -c dev:web` generates.
# Logs land in $TONK_RUN_DIR (default /tmp/tonk-run). Idempotent-ish: kill
# the old listeners first (see SKILL.md).
set -euo pipefail
ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
RUN="${TONK_RUN_DIR:-/tmp/tonk-run}"
mkdir -p "$RUN" "$HOME/.local/bin"
export PATH="$HOME/.local/bin:$PATH"

# Tools the flake would provide. Versions: trunk as in CI; wasm-bindgen
# must equal the `wasm-bindgen = "=…"` pin in the workspace Cargo.toml.
WB="$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$ROOT/Cargo.toml")"
if ! command -v trunk >/dev/null; then
  curl -sSL https://github.com/trunk-rs/trunk/releases/download/v0.21.14/trunk-x86_64-unknown-linux-gnu.tar.gz \
    | tar xz -C "$HOME/.local/bin"
fi
if [ "$(wasm-bindgen --version 2>/dev/null | cut -d' ' -f2)" != "$WB" ]; then
  curl -sSL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$WB/wasm-bindgen-$WB-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz -C "$RUN"
  cp "$RUN/wasm-bindgen-$WB-x86_64-unknown-linux-musl/wasm-bindgen" "$HOME/.local/bin/"
fi

cd "$ROOT"
cargo run -q --bin tonk-access-local --features helpers >"$RUN/access.log" 2>"$RUN/access.err" &
until ORIGIN="$(sed -n 's|^ACCESS_SERVICE_URL=||p' "$RUN/access.log" | head -n1)" && [ -n "$ORIGIN" ]; do sleep 2; done
echo "access service: $ORIGIN"

cp rust/tonk-ui/Trunk.toml rust/tonk-ui/.Trunk.dev.toml
for path in "/@" "/.well-known/tonk" "/.well-known/did.json" "/customer/"; do
  printf '\n[[proxies]]\nbackend = "%s%s"\n' "$ORIGIN" "$path" >>rust/tonk-ui/.Trunk.dev.toml
done
cp rust/tonk-ui/index.html rust/tonk-ui/.index.dev.html
nohup trunk serve "$PWD/rust/tonk-ui/.index.dev.html" --html-output index.html \
  --config rust/tonk-ui/.Trunk.dev.toml --proxy-backend "$ORIGIN/ucan/" >"$RUN/trunk.log" 2>&1 &
until grep -qE "INFO success|^error" "$RUN/trunk.log"; do sleep 10; done
tail -1 "$RUN/trunk.log"
echo "serving http://localhost:8080/"
