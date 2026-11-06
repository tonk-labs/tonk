# Linking progress and absence rendering (WEB-10)

Evidence: user sees a concept mismatch before approval, then stale waiting-for-
passkey text after signing. The supplied trace completes signing in 571 ms,
account publication in 22,886 ms, and authorization work in 23,670 ms. The user
reports completion seems to depend on keeping the page focused.

Hypothesis confirmed in source: overlay writes only schedule subscription polls.
Custody messages report working/done outside the command dispatcher, so their
reports need to broadcast explicitly. A regression must consume subscription
frames without manually polling or triggering another request.

The ceremony display also requires an explicit no-entity slot: no request status
exists before approval. Keep real errors visible; handle only this expected absence.

Steps: reproduce subscription failure; repair and rerun browser worker test;
add absence slot and run approval/UI checks; document WEB-10 and publish a PR.
Do not infer that these changes speed up the account push or fully explain
focus-dependent callback delivery. Those remain separate live-validation boundaries.

Results:
- Browser-worker regression failed before the fix: report returned without a
  subscriber frame. With explicit polling, both browser authorization tests pass.
- Seven focused approval UI tests pass, including the no-entity fallback.
- Full JavaScript test suite: 187 passed.
- No attempt was made to skip required registration publication or to claim its
  measured latency was removed. Focus-dependent callback delivery still needs
  live testing on the preview.
- Browser display tests: 15 state/slot tests and 1 stale no-entity diagnostic
  regression passed. Formatting and Storybook freshness/links passed.

Commands:
- `node --test rust/tonk-ui/tests/*.test.mjs`
- `cargo test --locked --offline --target wasm32-unknown-unknown -p tonk-worker --lib router::ceremony::authorization_tests`
- `cargo test --locked --offline --target wasm32-unknown-unknown -p tonk-display --lib no_entity`
- `cargo test --locked --offline --target wasm32-unknown-unknown -p tonk-display --lib state::`
- `cargo fmt --all --check`
- `git diff --check`
- `python3 docs/storybook/scripts/build.py --check`
- `python3 docs/storybook/scripts/check-links.py docs/storybook`

Wasm runs used installed wbg-pool with Chrome and wasm-bindgen 0.2.126.
