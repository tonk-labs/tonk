# Running the wasm browser tests without the Nix shell

`.cargo/config.toml` names `wbg-pool` as the wasm32 test runner, and the
Nix dev/CI shells put it on `PATH`. A plain checkout has no `wbg-pool`,
so `cargo test --target wasm32-unknown-unknown` fails before a test runs
— which is how a wasm-only module ends up shipped with nothing ever
having executed against it.

`wbg-pool` is not magic infrastructure, though: it is a crate in
`dialog-db`, which this workspace already depends on. Build it.

## The runner

```sh
cargo install --path <dialog-db>/rust/wbg-pool --root /tmp/wbg
export PATH="/tmp/wbg/bin:$PATH"
export CHROME=/opt/pw-browsers/chromium-*/chrome-linux/chrome   # any Chrome/Chromium
```

That is the whole setup. `.cargo/config.toml` already points the runner
at it, so:

```sh
cargo test    -p tonk-display --target wasm32-unknown-unknown
cargo nextest run -p tonk-display --target wasm32-unknown-unknown   # how CI runs it
```

No chromedriver, no `CARGO_TARGET_*_RUNNER` override, no browser on
`PATH` under a particular name — `CHROME` (or `WBG_POOL_BROWSER`) is
enough. The first invocation starts a daemon holding one headless
Chrome; it exits after five idle minutes.

**Prefer this over the stock runner**, and not only because it is what
CI uses: every test gets its own `t-<n>.localhost` origin, so IndexedDB,
service workers, `customElements` and the document are pristine per
test. Tests that install document-level listeners — anything driving the
real host — stop interfering with each other by construction.

## Writing a test that needs a host

A consumer element reaches IO by dispatching `tonk-query` (and friends)
and reading back `detail.result`, with a host claiming the event via
`preventDefault()`.

A test can stand one up by listening on `document`, but every consumer
event in the realm bubbles there, so such a listener must:

- **stand down when `event.defaultPrevented` is already set**, or it
  overwrites results another test's element-level stub has answered; and
- **claim only the query shapes it serves** — answering an unrelated
  component's query changes how that component behaves.

`registry.rs`'s `install_fake_host` is the worked example of both. Under
`wbg-pool`'s per-test origins neither can bite across tests, but both
still matter within one.

One more trap: `serde_wasm_bindgen::to_value` renders a Rust map as a JS
`Map`, whose contents `JSON.stringify` as `{}`. A fake host inspecting a
query body must read it back with `serde_wasm_bindgen::from_value`.

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

- **There is no service worker.** A DOM test cannot install one, so the
  router runs in-page: the same code over the same interface, one
  process boundary short. Nothing else in the path is stood in for.
- **A consumer element needs its own `with` attribute.** `resolve_with`
  reads it off the element itself, not its ancestors, and the host's
  observer that stamps descendants runs on a later task — so anything
  that subscribes the moment it is created has to carry the context
  over itself.

Assert a negative with a short bounded wait, not by polling until a
timeout: under one process per test, an exhausted poll costs seconds of
pure waiting per test. `registry.rs` keeps `settle_briefly` for this.

## If you must use the stock runner

`wasm-bindgen-test-runner` works, with more setup and no per-test
isolation. Three things have to line up, and each fails obscurely:

1. **A runner matching the pinned `wasm-bindgen`** — the version in
   `Cargo.lock`, exactly; it refuses a schema it does not know.
2. **A chromedriver matching the browser's MAJOR version.** A mismatch
   fails with a bare `Error: http status: 404` naming neither version.
   Chrome for Testing publishes a driver per exact version.
3. **A browser chromedriver can find.** `webdriver.json` is the
   documented way to set `goog:chromeOptions.binary`, but the runner
   looks for it somewhere other than the workspace root and silently
   falls back to defaults (`Try find webdriver.json …` / `Not found`,
   then `cannot find Chrome binary`). Symlinking the browser onto `PATH`
   as `google-chrome` / `chromium` / `chrome` is more reliable.

```sh
RUST_LOG=wasm_bindgen_cli=debug,ureq=debug cargo test … 2>&1 | grep "got:"
```

The `got: {…}` line carries chromedriver's own error, which is what says
`cannot find Chrome binary` or names a version mismatch.
