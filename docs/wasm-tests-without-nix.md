# Running the wasm browser tests without the Nix shell

`.cargo/config.toml` names `wbg-pool` as the wasm32 test runner. It is
provided by the Nix dev/CI shells and is not in this workspace, so a
plain checkout cannot run `cargo test --target wasm32-unknown-unknown`
at all — which is how a wasm-only module ends up shipped with no test
ever having executed against it.

The stock `wasm-bindgen-test-runner` works in its place. Three things
have to line up.

## 1. A runner matching the pinned `wasm-bindgen`

```sh
cargo install wasm-bindgen-cli --version "$(
  grep -A 1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p'
)" --root /tmp/wbg
```

The version must match `Cargo.lock` exactly; the runner refuses a
schema it does not recognise.

## 2. A chromedriver matching the browser's MAJOR version

This is the one that wastes time. A mismatched driver fails with a bare
`Error: http status: 404` that names neither version. Check both:

```sh
chromedriver --version
/opt/pw-browsers/chromium-*/chrome-linux/chrome --version
```

If the majors differ, fetch the matching driver — Chrome for Testing
publishes one per exact version:

```sh
curl -L -o cd.zip \
  "https://storage.googleapis.com/chrome-for-testing-public/<VERSION>/linux64/chromedriver-linux64.zip"
unzip cd.zip
```

## 3. A browser chromedriver can FIND

`webdriver.json` is the documented way to set `goog:chromeOptions.binary`,
but the runner looks for it somewhere other than the workspace root and
silently falls back to defaults — it prints `Try find webdriver.json …`
then `Not found`, and chromedriver reports `cannot find Chrome binary`.
Putting the browser on `PATH` under a name chromedriver probes for is
more reliable:

```sh
mkdir -p /tmp/browserbin
for n in google-chrome google-chrome-stable chrome chromium chromium-browser; do
  ln -sf /opt/pw-browsers/chromium-1194/chrome-linux/chrome /tmp/browserbin/$n
done
```

## Running

```sh
PATH="/tmp/browserbin:/tmp/wbg/bin:$PATH" \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
CHROMEDRIVER=/path/to/matching/chromedriver \
cargo test -p tonk-display --target wasm32-unknown-unknown --lib
```

## Diagnosing

The runner swallows the real cause behind a backtrace. `RUST_LOG` gets
it back:

```sh
RUST_LOG=wasm_bindgen_cli=debug,ureq=debug cargo test … 2>&1 | grep -i "session\|got:"
```

The `got: {…}` line carries chromedriver's own error message, which is
what says `cannot find Chrome binary` or names a version mismatch.

## Writing a test that needs a host

A consumer element reaches IO by dispatching `tonk-query` (and friends)
and reading back `detail.result`, with a host claiming the event via
`preventDefault()`. A test supplies its own host by listening on
`document` — but every query in the realm bubbles there, so the listener
must **stand down when `event.defaultPrevented` is already set**, or it
will overwrite results other tests' element-level stubs have answered.
`registry.rs`'s `install_fake_host` is the worked example.

One more trap: `serde_wasm_bindgen::to_value` renders a Rust map as a JS
`Map`, whose contents `JSON.stringify` as `{}`. A fake host inspecting a
query body must read it back with `serde_wasm_bindgen::from_value`, not
stringify it.

## Testing against a real worker

`tonk_worker::helpers` exposes what a full-stack browser test needs:
`state::test_state()` for a real `TonkState` (IndexedDB storage, a
profile, an attached account) and `serve::install_fetch(router)` to
answer `/api/...` in-page through the same browser<->axum conversion the
service worker runs. With those, the real `tonk-host` talks to a real
router over real `Request`/`Response` pairs, streamed bodies included —
so subscriptions work, not just one-shot queries.

`rust/tonk-display/tests/registry_fullstack.rs` is the worked example.
Two things it has to get right:

- **A test that installs the real host needs its own test BINARY.** The
  host claims every consumer event on the document, so sharing a page
  with tests that stub their own host means answering their queries too.
  wasm-bindgen gives each binary its own page; that is the only reliable
  isolation.
- **There is no service worker.** A DOM test cannot install one, so the
  router runs in-page: the same code over the same interface, one
  process boundary short. Nothing else in the path is stood in for.

A consumer element also needs its own `with` attribute — `resolve_with`
reads it off the element itself, not its ancestors; the host's observer
that stamps descendants runs on a later task.
