#!/bin/sh
set -eu
umask 077
cp /etc/cloudflare/certs/cloudflare-containers-ca.crt /usr/local/share/ca-certificates/cloudflare-containers-ca.crt
update-ca-certificates >/dev/null
export TONK_WORKER_BINARY=/usr/local/bin/tonk-worker-host
exec node /app/worker-hosted-server.mjs
