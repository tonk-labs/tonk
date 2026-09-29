# Duplicate spaces

Copy the source main branch's current durable data, definitions, and indexed blobs
into a freshly created repository. Do not carry history, other branches, source
credentials/delegations, membership, repository metadata, remotes, or seed revision
bookkeeping. Keep content entity identities and references intact. Generate normal
new-space ownership and metadata; use the normal account sync policy.

- [x] Implement and test snapshot duplication through the creation command.
- [x] Add a Hub duplicate dialog using existing creation receipts and busy state.
- [x] Add UI coverage, run focused checks, and record verification limits.

Validation completed:

- Three feature regressions pass: snapshot/blob preservation with fresh identity
  and metadata, missing-source refusal, and profile-only command dispatch.
- `cargo test -p tonk-worker --lib duplicate --quiet` passes (3 matches, including
  the existing route-table test); both `duplication::tests` also pass from the
  freshly compiled test binary.
- `cargo check -p tonk-ui --features integration-tests --tests --quiet` passes.
- Fresh `NO_COLOR=true trunk build --offline` passes. Final browser artifact:
  `7a7e9e9f7a6d770a`.
- The new Hub duplication test and existing collection-card creation test both
  pass with Chrome 154 and matching ChromeDriver against that artifact. The
  already-compiled test binary was rerun after the final YAML-only binding fix.
- `cargo fmt --all --check` and `git diff --check` pass.

Resolved validation failures: the native profile store needed sandbox escalation;
first browser startup raced the unfinished artifact; Chrome 150 crashed before
reaching the Hub. Chrome 154 exposed a literal default-name regression: `value`
sets the input property, but `form.reset()` restores its raw attribute. Using
`html:value` binds the default value, and the browser test now passes.

Not run: full workspace suite, hosted/remote hydration and sync, cross-device or
mobile UI validation. Storage failures after allocation retain the existing
creation behavior: an unfinished space can remain and the receipt reports failure.
