#!/usr/bin/env bash

set -euo pipefail

action_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/tonk-signing-test.XXXXXX")"
mock_dir="$test_dir/bin"
state_dir="$test_dir/state"
runner_temp="$test_dir/runner"

cleanup() {
  rm -rf "$test_dir"
}
trap cleanup EXIT

mkdir -p "$mock_dir" "$state_dir" "$runner_temp"
printf '%s\n' "/Users/runner/Library/Keychains/login.keychain-db" \
  > "$state_dir/search-list"
printf 'unsigned binary' > "$test_dir/tonk"

cat > "$mock_dir/security" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

command_name="$1"
shift

case "$command_name" in
  create-keychain)
    keychain_path="${!#}"
    printf '%s\n' "$keychain_path" > "$TEST_STATE_DIR/current-keychain"
    ;;
  list-keychains)
    if [[ "${1:-}" == "-d" ]]; then
      shift 2
    fi
    if [[ "${1:-}" == "-s" ]]; then
      shift
      printf '%s\n' "$@" > "$TEST_STATE_DIR/search-list"
    else
      while IFS= read -r keychain; do
        printf '    "%s"\n' "$keychain"
      done < "$TEST_STATE_DIR/search-list"
    fi
    ;;
  find-identity)
    printf '%s\n' \
      '  1) 2DA41BBEFF8B18B3CEDACFF92F211581F6ABA52E "Developer ID Application: Tonk Labs"'
    ;;
  delete-keychain|import|set-keychain-settings|set-key-partition-list|unlock-keychain)
    ;;
  *)
    printf 'unexpected security command: %s\n' "$command_name" >&2
    exit 1
    ;;
esac
EOF

cat > "$mock_dir/codesign" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

check_notarization=false
notarized_requirement=false
for argument in "$@"; do
  case "$argument" in
    --check-notarization) check_notarization=true ;;
    -R=notarized) notarized_requirement=true ;;
  esac
done

if [[ "$check_notarization" == true ]]; then
  if [[ "$notarized_requirement" != true ]]; then
    printf 'notarization check did not require a notarized ticket\n' >&2
    exit 1
  fi
  touch "$TEST_STATE_DIR/notarization-checked"
  exit 0
fi

for argument in "$@"; do
  [[ "$argument" == "--verify" ]] && exit 0
done

keychain_path="$(cat "$TEST_STATE_DIR/current-keychain")"
if ! grep -Fxq "$keychain_path" "$TEST_STATE_DIR/search-list"; then
  printf '%s: no identity found\n' \
    '2DA41BBEFF8B18B3CEDACFF92F211581F6ABA52E' >&2
  exit 1
fi
EOF

cat > "$mock_dir/ditto" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output_path="${!#}"
: > "$output_path"
EOF

cat > "$mock_dir/xcrun" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '{"status":"Accepted","id":"test-submission"}\n'
EOF

cat > "$mock_dir/spctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' \
  'artifacts/tonk: rejected (the code is valid but does not seem to be an app)' >&2
exit 3
EOF

chmod +x "$mock_dir/security" "$mock_dir/codesign" "$mock_dir/ditto" \
  "$mock_dir/xcrun" "$mock_dir/spctl"

awk '
  /^      run: \|$/ { in_run = 1; next }
  in_run { sub(/^        /, ""); print }
' "$action_dir/action.yml" > "$test_dir/action.sh"

export TEST_STATE_DIR="$state_dir"
export PATH="$mock_dir:/usr/bin:/bin:/usr/sbin:/sbin"
export RUNNER_OS=macOS
export RUNNER_TEMP="$runner_temp"
export BINARY_PATH="$test_dir/tonk"
export CERTIFICATE_P12=ZHVtbXk=
export CERTIFICATE_PASSWORD=test-password
export APP_STORE_CONNECT_KEY=test-key
export APP_STORE_CONNECT_KEY_ID=TESTKEY123
export APP_STORE_CONNECT_ISSUER_ID=00000000-0000-0000-0000-000000000000

bash "$test_dir/action.sh"

if [[ ! -f "$state_dir/notarization-checked" ]]; then
  printf 'notarization ticket was not verified for the CLI binary\n' >&2
  exit 1
fi

expected_keychain="/Users/runner/Library/Keychains/login.keychain-db"
actual_keychains="$(cat "$state_dir/search-list")"
if [[ "$actual_keychains" != "$expected_keychain" ]]; then
  printf 'keychain search list was not restored after signing\n' >&2
  exit 1
fi
