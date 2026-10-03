#!/usr/bin/env bash
set -euo pipefail

action_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "$action_dir/../../.." && pwd)"
test_dir="$(mktemp -d "${TMPDIR:-/tmp}/tonk-linux-smoke.XXXXXX")"
trap 'rm -rf "$test_dir"' EXIT

if [[ "${1:-}" == --binary ]]; then
  mkdir -p "$test_dir/source"
  cp -R "$repo_dir/rust/tonk-cli/npm/cli" "$test_dir/source/cli"
  cp -R "$repo_dir/rust/tonk-cli/npm/linux-x64" "$test_dir/source/linux-x64"
  mkdir -p "$test_dir/source/linux-x64/bin"
  cp "$2" "$test_dir/source/linux-x64/bin/tonk"
  chmod 0755 "$test_dir/source/linux-x64/bin/tonk"
  for package in cli linux-x64; do
    npm pack "$test_dir/source/$package" --cache "$test_dir/npm-cache" \
      --pack-destination "$test_dir" --json \
      > "$test_dir/$package.json"
  done
  wrapper_name="$(node -p "require(process.argv[1])[0].filename" "$test_dir/cli.json")"
  linux_name="$(node -p "require(process.argv[1])[0].filename" "$test_dir/linux-x64.json")"
  mv "$test_dir/$wrapper_name" "$test_dir/wrapper.tgz"
  mv "$test_dir/$linux_name" "$test_dir/linux.tgz"
  expected_version="${3:-}"
else
  # Release mode tests the exact tarballs that npm publish will receive.
  cp "$1" "$test_dir/wrapper.tgz"
  cp "$2" "$test_dir/linux.tgz"
  expected_version="${3:-}"
fi

for distro in bookworm-slim alpine; do
  image="tonk-linux-smoke:$distro"
  docker build --platform linux/amd64 \
    --build-arg "BASE_IMAGE=node:22-$distro" \
    --tag "$image" "$action_dir"
  docker run --rm --platform linux/amd64 --network none \
    --mount "type=bind,source=$test_dir,target=/packages,readonly" \
    --mount "type=bind,source=$action_dir/container.sh,target=/smoke.sh,readonly" \
    --env "EXPECTED_VERSION=$expected_version" "$image" sh /smoke.sh
done
