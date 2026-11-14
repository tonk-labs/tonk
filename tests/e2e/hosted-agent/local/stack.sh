#!/usr/bin/env bash
# The hosted agent on this machine: the native access service, the app
# under `trunk serve`, and the MCP Worker under `wrangler dev`, all on one
# origin (http://localhost:8081) through proxy.mjs. Then, in another shell:
#
#   cd tests/e2e/hosted-agent
#   TONK_E2E_ORIGIN=http://localhost:8081 \
#   TONK_E2E_CAPTURED_EMAILS="$(cat "$RUN/access-url")" npm test
#
# Needs trunk and wasm-bindgen (`.claude/skills/run-app/dev-web.sh` installs
# both, and writes the trunk config this extends) and `worker-build`.
set -euo pipefail
ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
HERE="$(cd "$(dirname "$0")" && pwd)"
RUN="${TONK_RUN_DIR:-/tmp/tonk-run}"
ORIGIN=http://localhost:8081
mkdir -p "$RUN"
cd "$ROOT"
trap 'kill 0' EXIT

cargo build -q --bin tonk-access-local --features helpers
(cd rust/tonk-host-worker && worker-build --release >"$RUN/host-build.log" 2>&1)

ACCESS_PUBLIC_ORIGIN="$ORIGIN" target/debug/tonk-access-local >"$RUN/access.log" 2>"$RUN/access.err" &
until ACCESS="$(sed -n 's|^ACCESS_SERVICE_URL=||p' "$RUN/access.log" | head -n1)" && [ -n "$ACCESS" ]; do sleep 1; done
echo "$ACCESS" >"$RUN/access-url"

# The app's own proxies, pointed at this run's access service.
cp rust/tonk-ui/Trunk.toml rust/tonk-ui/.Trunk.dev.toml
for path in "/@" "/.well-known/tonk" "/.well-known/did.json" "/customer/"; do
  printf '\n[[proxies]]\nbackend = "%s%s"\n' "$ACCESS" "$path" >>rust/tonk-ui/.Trunk.dev.toml
done
cp rust/tonk-ui/index.html rust/tonk-ui/.index.dev.html
trunk serve "$PWD/rust/tonk-ui/.index.dev.html" --html-output index.html \
  --config rust/tonk-ui/.Trunk.dev.toml --proxy-backend "$ACCESS/ucan/" >"$RUN/trunk.log" 2>&1 &

(cd rust/tonk-host-worker && npx --yes wrangler@4.128.0 dev --port 8790 --ip 127.0.0.1 \
  --var MCP_SESSION_SECRET:local-e2e --var "ACCESS_SERVICE_URL:$ORIGIN" >"$RUN/host-dev.log" 2>&1) &

node "$HERE/proxy.mjs" "$ACCESS" >"$RUN/proxy.log" 2>&1 &

until grep -q "INFO success" "$RUN/trunk.log" && grep -q "Ready on" "$RUN/host-dev.log"; do sleep 5; done
echo "serving $ORIGIN (access service $ACCESS)"
wait
