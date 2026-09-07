#!/usr/bin/env bash
set -euo pipefail

global_tmp=${TMPDIR:-/tmp}
created_root=0
if [[ -n "${TONK_E2E_STORAGE_TMPDIR:-}" ]]; then
  test_root=$TONK_E2E_STORAGE_TMPDIR
  mkdir -p "$test_root"
  if find "$test_root" -mindepth 1 -print -quit | grep -q .; then
    echo "test:storage: supplied root must be empty: $test_root" >&2
    exit 2
  fi
else
  test_root=$(mktemp -d "$global_tmp/tonk-e2e-storage.XXXXXX")
  created_root=1
fi

case "$test_root" in
/*) ;;
*)
  echo "test:storage: isolated root must be absolute: $test_root" >&2
  exit 2
  ;;
esac

bytes_below() {
  local path=$1
  local kib
  kib=$(du -sk "$path" | awk '{print $1}')
  printf '%s' "$((kib * 1024))"
}

scoped_profile_count() {
  find "$global_tmp" -maxdepth 1 -type d -name 'org.chromium.Chromium.scoped_dir.*' 2>/dev/null |
    wc -l |
    tr -d ' '
}

initial_bytes=$(bytes_below "$test_root")
initial_scoped_profiles=$(scoped_profile_count)
echo "test:storage: isolated root: $test_root"
echo "test:storage: initial bytes: $initial_bytes"

set +e
TMPDIR="$test_root" cargo test --locked -p tonk-ui --features integration-tests \
  identity::tests::it_serves_deployment_config_on_the_page_origin -- --test-threads=1
test_status=$?
set -e

retained_paths=$(find "$test_root" -mindepth 1 \
  \( -name 'org.chromium.Chromium.scoped_dir.*' \
  -o -name 'tonk-e2e-*' \
  -o -name '.tonk-e2e-workspace' \) -print)
status=0
retained_processes=""
set +e
candidate_pids=$(pgrep -f 'chromedriver|Google Chrome|Chromium|chromium|caddy|tonk-ui-test-server')
pgrep_status=$?
set -e
if ((pgrep_status != 0 && pgrep_status != 1)); then
  echo "test:storage: pgrep failed with status $pgrep_status" >&2
  status=1
fi
for candidate_pid in $candidate_pids; do
  process=$(ps eww -p "$candidate_pid" -o pid= -o command= 2>/dev/null || true)
  if [[ "$process" == *"$test_root"* ]]; then
    retained_processes+="$process"$'\n'
  fi
done
retained_processes=${retained_processes%$'\n'}
final_bytes=$(bytes_below "$test_root")
final_scoped_profiles=$(scoped_profile_count)
if ((test_status != 0)); then
  echo "test:storage: focused browser test failed with status $test_status" >&2
  status=1
fi
if [[ -n "$retained_paths" ]]; then
  echo "test:storage: retained browser or Tonk workspace paths:" >&2
  printf '%s\n' "$retained_paths" >&2
  status=1
fi
if [[ -n "$retained_processes" ]]; then
  echo "test:storage: retained child processes associated with the isolated root:" >&2
  printf '%s\n' "$retained_processes" >&2
  status=1
fi
if ((final_scoped_profiles > initial_scoped_profiles)); then
  echo "test:storage: process-global Chrome scoped profiles increased from $initial_scoped_profiles to $final_scoped_profiles" >&2
  status=1
fi
if ((final_bytes != initial_bytes)); then
  echo "test:storage: isolated root grew from $initial_bytes to $final_bytes bytes" >&2
  find "$test_root" -mindepth 1 -print >&2
  status=1
fi

if ((status == 0)); then
  echo "test:storage: passed with no retained test artifacts"
  if ((created_root == 1)); then
    rmdir "$test_root"
  fi
else
  echo "test:storage: retained diagnostic root: $test_root" >&2
fi

exit "$status"
