#!/usr/bin/env bash
# Known-good targeted edit: query-bind the existing habit, overwrite
# its single-valued (`:last`) name.
set -euo pipefail
TONK="${TONK:?}"

"$TONK" eval -c 'habit:
  this: ?h
  name: "Inbox zero"

habit!:
  this: ?h
  name: "Inbox Zero — daily"'

"$TONK" status
