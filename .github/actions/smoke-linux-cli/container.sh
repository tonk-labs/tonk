#!/bin/sh
set -eu

test ! -e /nix/store
npm install --global --prefix /tmp/tonk --offline --ignore-scripts \
  --no-audit --no-fund /packages/wrapper.tgz /packages/linux.tgz

binary=/tmp/tonk/lib/node_modules/@tonk/cli-linux-x64/bin/tonk
readelf --file-header "$binary"
readelf --program-headers --wide "$binary" > /tmp/program-headers
readelf --dynamic --wide "$binary" > /tmp/dynamic
cat /tmp/program-headers /tmp/dynamic

# A standalone static binary has no loader, shared libraries, or search path.
if grep -Eq 'INTERP|Requesting program interpreter' /tmp/program-headers \
  || grep -Eq '\((NEEDED|RPATH|RUNPATH)\)' /tmp/dynamic; then
  echo 'Linux CLI must not depend on a dynamic loader or shared libraries' >&2
  exit 1
fi
if ! readelf --file-header "$binary" | grep -q 'Advanced Micro Devices X86-64'; then
  echo 'Expected an x86-64 Linux executable' >&2
  exit 1
fi

/tmp/tonk/bin/tonk --version > /tmp/version
cat /tmp/version
if [ -n "${EXPECTED_VERSION:-}" ]; then
  grep -Fx "tonk $EXPECTED_VERSION" /tmp/version
fi
/tmp/tonk/bin/tonk --help > /tmp/help
cat /tmp/help
grep -Eiq '^[[:space:]]*usage:' /tmp/help
