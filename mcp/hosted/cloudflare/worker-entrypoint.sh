#!/bin/sh
set -eu
umask 077
cp /etc/cloudflare/certs/cloudflare-containers-ca.crt /usr/local/share/ca-certificates/cloudflare-containers-ca.crt
update-ca-certificates >/dev/null
export TONK_AGENT_WORKER_BINARY=/usr/local/bin/tonk-agent-worker
exec node /app/hosted/server.mjs
