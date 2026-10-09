#!/bin/sh
set -eu
umask 077
cp /etc/cloudflare/certs/cloudflare-containers-ca.crt /usr/local/share/ca-certificates/cloudflare-containers-ca.crt
update-ca-certificates >/dev/null
exec node /app/hosted-server.mjs
