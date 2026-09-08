# Space deletion dialog

Fix the hosted-space deletion review after client-side navigation without a reload.

Evidence: `tonk-site` reuses its iframe on a path change, but the bridge sends
URL context only during its initial handshake. Settings reads that stale context.
The dialog also renders raw subjects for unnamed spaces and ownership errors.

- Publish fresh location context before re-claiming the route in a reused guest.
- Show a readable space name or an unnamed-space label, and use `delete space`
  throughout the space confirmation. Keep ownership checks and submission IDs.
- Wrap long names and separate the consequences into readable paragraphs.
- Verify the bridge update and settings dialog with focused browser Wasm tests.

Validation:
- Direct Chrome Wasm runner: 43/43 tonk-portal tests passed, including a
  regression that failed without the context update.
- Direct Chrome Wasm runner: 7/7 focused settings tests passed, including
  automatic opening and long-name scrollWidth/clientWidth coverage.
- Native-input contracts: 6/6 passed. `cargo fmt --all` and `git diff --check` pass.
- Pooled runner failed to start; direct `wasm-bindgen-test-runner` succeeded with
  `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
  and `WASM_BINDGEN_USE_BROWSER=1`.
- Full signed-in Hub journey and Safari remain unverified. No deletion submitted.
