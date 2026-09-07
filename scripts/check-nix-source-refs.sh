#!/usr/bin/env bash
set -euo pipefail

root=${1:-.}

if [[ ! -d "$root" ]]; then
  echo "check-nix-source-refs: root is not a directory: $root" >&2
  exit 2
fi

scan_roots=()
for candidate in README.md docs plan .github nix scripts; do
  if [[ -e "$root/$candidate" ]]; then
    scan_roots+=("$candidate")
  fi
done

if ((${#scan_roots[@]} == 0)); then
  exit 0
fi

pattern='\bnix[[:space:]]+(develop|build|run|flake[[:space:]]+check)\b[^\r\n]*path:\.'
set +e
matches=$(
  cd "$root" &&
    rg --line-number --with-filename --no-heading --color never \
      --glob '!**/.git/**' \
      --glob '!**/target/**' \
      --glob '!**/result/**' \
      --glob '!scripts/tests/check-nix-source-refs.sh' \
      "$pattern" "${scan_roots[@]}"
)
status=$?
set -e

case "$status" in
0)
  echo "Unsafe path-flake references found:" >&2
  printf '%s\n' "$matches" >&2
  exit 1
  ;;
1)
  exit 0
  ;;
*)
  echo "check-nix-source-refs: rg failed with status $status" >&2
  exit "$status"
  ;;
esac
