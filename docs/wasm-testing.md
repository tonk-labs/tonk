# Wasm browser testing

Tonk runs `wasm32-unknown-unknown` tests through `wbg-pool`. The runner keeps
one headless Chrome process alive and gives every nextest test a unique
`t-*.localhost` origin. Each test therefore receives fresh IndexedDB, OPFS,
local storage, caches, and service-worker registrations without paying for a
new browser process.

## Commands

Run the canonical pooled suites from the Nix shell:

```sh
nix develop --accept-flake-config path:. -c test:web:debug
nix develop --accept-flake-config path:. -c test:web:release
```

To reproduce a suspected runner discrepancy unchanged with the stock runner:

```sh
nix develop --accept-flake-config path:. -c test:web:debug:stock
nix develop --accept-flake-config path:. -c test:web:release:stock
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
nix develop --accept-flake-config path:. -c wbg-pool daemon --stop
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
