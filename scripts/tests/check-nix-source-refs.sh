#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/scripts/check-nix-source-refs.sh"
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/tonk-nix-source-refs.XXXXXX")
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/docs"

cat >"$fixture_root/docs/accepted.md" <<'EOF'
```sh
nix develop . -c test:web:debug
nix develop .#ci
```

Using a path-flake reference can copy ignored build products into the store.
EOF

"$checker" "$fixture_root"

assert_rejected() {
  local command=$1
  local expected_line=$2
  local output

  printf '%s\n' "$command" >"$fixture_root/docs/rejected.md"
  if output=$("$checker" "$fixture_root" 2>&1); then
    echo "expected checker to reject: $command" >&2
    exit 1
  fi
  if [[ "$output" != *"docs/rejected.md:1:"* ]] || [[ "$output" != *"$expected_line"* ]]; then
    echo "checker did not report the offending file, line, and command" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
  rm "$fixture_root/docs/rejected.md"
}

assert_rejected 'nix develop path:. -c test:web:debug' 'nix develop path:.'
assert_rejected "nix flake check 'path:.'" "nix flake check 'path:.'"
assert_rejected 'nix build --accept-flake-config path:.#x' 'nix build --accept-flake-config path:.#x'

echo "check-nix-source-refs fixture tests passed"
