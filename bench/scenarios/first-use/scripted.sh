#!/usr/bin/env bash
# Known-good three-call trajectory: orient, live read, precise update.
set -euo pipefail

"${TONK:?}" guide
"$TONK" query task
"$TONK" assert task launch-email --done true
