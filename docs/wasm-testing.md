# Wasm browser testing

Tonk runs `wasm32-unknown-unknown` tests through `wbg-pool`. The runner keeps
one headless Chrome process alive and gives every nextest test a unique
`t-*.localhost` origin. Each test therefore receives fresh IndexedDB, OPFS,
local storage, caches, and service-worker registrations without paying for a
new browser process.

## Commands

Run the canonical pooled suites from the Nix shell:

```sh
nix develop --accept-flake-config . -c test:web:debug
nix develop --accept-flake-config . -c test:web:release
```

To reproduce a suspected runner discrepancy unchanged with the stock runner:

```sh
nix develop --accept-flake-config . -c test:web:debug:stock
nix develop --accept-flake-config . -c test:web:release:stock
```

These commands deliberately use the Git-backed flake source. Tracked working-tree
modifications are included, but newly created files must be staged before Nix can
evaluate them. Do not switch an ad-hoc command to a path-flake source: when an
ignored `target/` or another build tree is present, that source form can copy the
whole tree into the Nix store. Check repository-authored commands with:

```sh
bash scripts/check-nix-source-refs.sh .
```

Both command families accept nextest arguments such as `-E 'package(tonk-ui)'`
and `--test-threads 1`. The stock commands clear pool-only environment
variables while retaining `CHROME` and `CHROMEDRIVER` for diagnosis.

The checked-in Cargo configuration also selects `wbg-pool` for direct Wasm
Cargo commands. Run those commands inside `nix develop`; outside the Nix shell,
the runner may not be on `PATH`.

## Browser and platform setup

The runner is Unix-only. The Linux CI shell supplies Chromium and sets
`WBG_POOL_NO_SANDBOX=1` because the trusted GitHub Actions environment cannot
initialize Chrome's sandbox. On macOS the shell expects Google Chrome at:

```text
/Applications/Google Chrome.app/Contents/MacOS/Google Chrome
```

Override `WBG_POOL_BROWSER` inside the shell or for the test invocation when
Chrome is installed elsewhere. `CHROME` is also accepted on Linux.

`wbg-pool` delegates Node and Emscripten test modes to the pinned
`wasm-bindgen-test-runner`. Benches and coverage dumps are not supported by the
pool and must use the stock runner directly.

## Version and daemon lifecycle

The runner's `wasm-bindgen-cli-support` version must match the workspace's
`wasm-bindgen` version. Tonk builds `wbg-pool` from the pinned dialog-db source
and pins its fallback runner in the same Nix shell; update those versions
together.

The daemon starts on demand and exits after its idle timeout. Its rendezvous
state uses `WBG_POOL_DIR`, then `$XDG_RUNTIME_DIR/wbg-pool`, then a directory
under `$TMPDIR`. Stop it through its supported command when diagnosing runner
lifecycle issues:

```sh
nix develop --accept-flake-config . -c wbg-pool daemon --stop
```

One daemon serves one bindgen version. Set a checkout-specific `WBG_POOL_DIR`
when concurrently testing worktrees that pin different versions.

## Diagnosing discrepancies

Do not change product code from a pooled-only failure. Rerun the same archive,
filter, thread count, retry count, and profile with the matching `:stock`
command. Classify differences in runner startup, browser lifecycle, timeout,
console capture, or storage isolation before editing a test or restoring a
platform-wide retry. Canonical Wasm CI is intentionally single-attempt
evidence; tests that genuinely need retries must scope them explicitly.

## Browser test storage

The real-browser harness owns one `tonk-e2e-*` workspace per server lifecycle.
Caddy state, the mutable service-worker fixture, ChromeDriver logs, and each
unique Chrome profile all live below that root. An orderly stop terminates the
children before deleting the workspace; early failures leave at most that
identifiable Tonk-owned root for diagnosis.

Run the focused storage regression twice when changing this lifecycle:

```sh
nix develop . -c test:storage
nix develop . -c test:storage
```

Read-only inspection is safe:

```sh
find "${TMPDIR:-/tmp}" -maxdepth 1 -type d \
  \( -name 'tonk-e2e-*' -o -name 'org.chromium.Chromium.scoped_dir.*' \) -print
ps eww -ax -o pid= -o command= | rg 'tonk-e2e-|chromedriver|Google Chrome|Chromium|caddy'
```

Cleanup is a separate, explicit recovery operation. Before removing anything,
resolve one exact path, verify that its name is Tonk-owned, inspect its owner and
age (`ls -ldT <exact-path>` on macOS or `stat <exact-path>` on Linux), and confirm
that no live process references it. Remove only that validated path. Never clear
a normal Chrome profile, Tonk storage, IndexedDB, Cache Storage, or passkeys.

## Local Cargo artifacts

The test profile disables incremental compilation and embedded debuginfo to keep
repeated native and Wasm test builds bounded. This makes stack-level debugging
less detailed; opt into symbols for one diagnostic invocation with
`CARGO_PROFILE_TEST_DEBUG=1`.

Measure a clean, explicit target without touching the checkout's normal
`target/` directory:

```sh
mkdir /tmp/tonk-test-storage-measurement
CARGO_TARGET_DIR=/tmp/tonk-test-storage-measurement \
  bash scripts/measure-cargo-test-storage.sh
du -sk /tmp/tonk-test-storage-measurement/{debug/incremental,debug/deps,wasm32-unknown-unknown} 2>/dev/null
```

The measurement script requires the target directory to be empty and reports
stable `key=value` byte counts, including repeat growth. Cleanup remains a
separate decision: after validating the exact path is the disposable measurement
directory, use `cargo clean --target-dir <validated-path>`. Never run a broad
cleanup across worktrees as routine test recovery.
