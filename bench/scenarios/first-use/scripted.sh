#!/usr/bin/env bash
# Current agent workflow; the write receipt supplies local verification.
set -euo pipefail

"${TONK:?}" space agents get
"$TONK" show
"$TONK" query task --where 'title=Draft launch email' --where done=false --json
"$TONK" assert task launch-email --done true --json
