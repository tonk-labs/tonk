#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
app="$PWD/.build/Tonk.app"
if [[ ! -d "$app" ]]; then
    echo "Build the app first: bash scripts/build-app.sh" >&2
    exit 1
fi
mkdir -p artifacts
run_dir=$(mktemp -d "$PWD/artifacts/smoke.XXXXXX")
report="$run_dir/report.json"
open -n "$app" --args --smoke-test --data-dir "$run_dir/data" --report "$report" "$@"
for ((attempt = 0; attempt < 80; attempt++)); do
    if [[ -f "$report" ]]; then
        cat "$report"
        python3 - "$report" <<'PY'
import json,sys
with open(sys.argv[1]) as source:
    report = json.load(source)
sys.exit(0 if report.get('passed') else 1)
PY
        exit $?
    fi
    sleep 1
done
echo "Native smoke test timed out; report expected at $report" >&2
exit 1
