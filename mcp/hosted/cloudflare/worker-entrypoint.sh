#!/bin/sh
set -eu
umask 077
cp /etc/cloudflare/certs/cloudflare-containers-ca.crt /usr/local/share/ca-certificates/cloudflare-containers-ca.crt
update-ca-certificates >/dev/null
# The OS store covers the Rust worker (rustls-tls-native-roots), but Node's
# fetch only trusts its bundled roots. Without this, expanding a /@/ agent
# shortcut through the intercepting egress fails TLS verification.
export NODE_EXTRA_CA_CERTS=/etc/cloudflare/certs/cloudflare-containers-ca.crt
export TONK_AGENT_WORKER_BINARY=/usr/local/bin/tonk-agent-worker
exec node /app/hosted/server.mjs
