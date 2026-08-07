#!/usr/bin/env bash
# Print the canonical list-append example out of the shipped binary's own
# guide, so scenarios evaluate exactly the text an agent reads.
#
# `tonk guide events` holds the one copy of that example; everything else
# extracts it. The awk program below is duplicated verbatim in
# rust/tonk-cli/tests/notation.rs, which asserts the extracted block
# evaluates and commits — keep the two in step.
set -euo pipefail

TONK="${TONK:?}"

"$TONK" guide events | awk '
  /^```yaml$/ { fences++ }
  fences == 1 && !/^```/ { print }
  /^```$/ { if (fences == 1) exit }
'
