# Legacy account UI migration

Base: staging `d69c737ef` (account views / account branches stack).

Investigate signed-in UI failures without resetting account data. Reproduce
upgrading the actual pre-stack profile library, then fix the smallest proven
compatibility gap. Preserve account facts and authored content; verify repeat
startup is idempotent. Publish a reviewed PR to staging after focused checks.

## Reproduction

The reported browser shows an oversized wordmark, unstyled space links, and
missing account controls. The user confirms it follows old accounts across
browsers. The account's actual data has not been exported or modified here.

Using the exact shipped `profile.yaml` at `95fea7462`, an unrecorded install
fails reconciliation with:

> `{profile-branch}` is not a field of `tonk:space/chrome` — it renders as
> nothing. `tonk:space/chrome` declares `id`, `rest`.

A single recorded install upgrades successfully because its schemas can be
retracted before evaluation. Without usable provenance (or when a retained
schema is absent from the latest install delta), analysis reads the old branch
schema while validating the replacement view. Startup logs the failure and
keeps the obsolete UI.

## Migration

The reconciler already prepares the complete desired library with isolated
analysis. Apply those assertions to the transaction overlay after provenance
retractions and before branch-backed evaluation. Schema, views, and install
record publish atomically through the existing bounded CAS retry path.

This runs automatically at startup and after account hydration/sync. It needs
no account reset or user action. Only shipped assertion addresses are replaced;
account data and unrelated authored content remain. Unattributed retired
definitions remain subject to the existing deferred-cleanup policy. An
exploratory test also found old routes surviving multiple install deltas; broad
historical cleanup is separate from making the current library usable.

## Validation

- Pre-fix: exact unrecorded historical library fails with the schema error above.
- Initial fix: recorded and unrecorded historical upgrade tests pass.
- Expanded tests cover intermediate upgrades, unusable provenance, renderer
  stylesheet bindings, preserved profile name/space/authored route, and a repeat
  reconciliation with the worker receipt cleared.
- `cargo test -p tonk-worker --lib`: 175 passed.
- `cargo test -p tonk-worker --target wasm32-unknown-unknown --lib profile_library_upgrades -- --nocapture`: 4 passed in the browser-backed runner.
- Formatting, diff whitespace, Storybook build freshness and local links pass.
- Initial expanded assertions incorrectly searched CBOR bytes as text; corrected
  to decode through `tonk_template::embed::Embeds` before the passing runs.
- Deployed browser recovery, Safari, and the complete login/UI journey remain
  unverified.
