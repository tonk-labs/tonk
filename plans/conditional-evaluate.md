# Conditional local evaluation

Expose POST /api/repository/{repo}/branch/{branch}/evaluate/conditional with JSON:
`{"document":"inline notation", "expected_revision": <revision object or null>}`.
The revision field is mandatory; null explicitly means an empty branch. The
ordinary evaluate route remains backward compatible. A dedicated route makes
older workers reject the request rather than ignore a condition and write.

The evaluator opens a private handle over the same local branch/storage so its
checked revision cannot be changed by a concurrent refresh of the reactor's
cached handle. This does not create another replica or perform sync. Conditional
builds read durable state, not the cached handle's ephemeral session overlay.
The existing staged publication CAS enforces the checked head at publication.
Never retry conditional writes after a CAS race: return HTTP 412, refresh the
cached reader and ask the caller to read/preview again. Successful commits refresh
the cached handle before subscription polling and normal dirty-queue processing.
Delivery or response failure after commit is an uncertain outcome, not authority
to repeat the document. Transient command documents are rejected before commit: a scoped build tool
does not dispatch account/runtime commands. DOM render completion remains
separate from the durable revision acknowledgment.

Tests cover a required revision field, matching apply and immediate cached
readback, stale rejection, competing writers with one winner, and a forced external
publish plus cached-handle refresh during evaluation. The latter specifically
protects against checking a mutable cached revision before an await and then
staging against a different head. Normal evaluation/seed retry behavior is kept.

Validation command:
`cargo test -p tonk-worker conditional_ --lib`
Also run the broader evaluate module and check wasm compilation before rollout.

Validation completed: 14 evaluator-module tests passed, followed by all six
conditional regressions after the final no-retry guard change. The HTTP test
confirms missing revision -> 422, successful conditional write -> 200, and stale
replay -> 412. The Wasm target checks successfully; cargo fmt and diff checks pass.
Builds emitted existing dead-code warnings in unrelated worker helpers.
This is native worker execution plus Wasm compilation, not a browser render test
or a deployment. No staging or production worker has been changed.

## PR #1055 CI repair

The initial CI run failed Clippy's `unnecessary_unwrap` in the competing-writers
regression. The end-to-end aggregate also failed because lint failure skipped
its shards. Replace the checked unwrap with `if let Err(error)`, preserving the
one-winner and precondition-failure assertions.

Validation: all six conditional tests passed with host access after sandboxed
execution aborted with `failed to initiate panic, error 5`. `cargo fmt --all --
--check`, `git diff --check`, and `nix --accept-flake-config develop .#ci -c lint`
passed. Nix verified all five checks on aarch64-darwin, including workspace
Clippy with all targets/features and warnings denied. Hosted Linux CI must
still run on the pushed commit.

The next hosted run passed lint, all end-to-end shards, and the five Wasm
conditional execution tests, but web-debug failed the pinned route-table guard:
the conditional endpoint was registered without its matching `ROUTES` entry.
Add that entry in sorted order. This is the existing evaluation data plane with
a revision precondition; a transient command cannot provide the required
pre-commit guard, and older workers must reject the distinct endpoint rather
than silently perform an unconditional write.

Validation after the table edit: both native `router::route_table` tests passed,
as did `cargo fmt --all -- --check` and `git diff --check`. The full hosted web
suite remains to be rerun on the new commit.
