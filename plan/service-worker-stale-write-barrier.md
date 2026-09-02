# Service-worker stale-write barrier implementation plan

**Goal:** Prevent a protocol-capable page or sealed guest from mutating Tonk through a service worker from another build while preserving reads, subscriptions, dry-run evaluation, and an explicit migration path for genuinely older contexts that cannot send a build stamp.

**Approach:** Derive immutable document build metadata from the outer service-worker policy plus its worker glue/Wasm, stamp it into `index.html` before the app can mount, and keep live `version.json` strictly as update discovery. Put top-document UI requests to the local `/api` worker surface behind one build-aware request adapter. The trusted portal normalizes each guest URL once, default-denies undeclared/control routes, enforces the portal's repository reach, strips caller provenance, and stamps only explicit worker data-plane requests. Replace the worker's suffix-only write check with a default-safe method-and-route policy: explicit read-like POST routes pass, known state-changing exceptions are pinned, and every other non-read method is treated as a write. Exact stale markers relay through nested hosts to the existing trusted top prompt. Activation retains generation caches, and withdrawal/failure recovery never deletes local artifacts or unregisters workers.

**Constraints:**

- Preserve local IndexedDB, CacheStorage, profiles, passkeys, and registrations. This change must not add any automatic clear, unregister, generation purge, or destructive reset; the existing guarded update-alignment/watchdog reload rules remain the only automatic reloads.
- Preserve an existing stale page's GET/HEAD reads, query POSTs/SSE subscriptions, and an evaluate request only when its parsed query contains exactly one lowercase `transact=false` value. The handler's looser `0`/`no`/case-insensitive aliases remain valid requests but are conservatively write-gated across a build mismatch.
- Treat missing `x-tonk-build` as unclassified and compatible for genuinely pre-protocol or development pages. Current generated pages and their sealed guests carry immutable provenance. Treat a present empty, non-text, duplicate, or otherwise invalid build header as a typed fail-closed error on a classified write.
- Treat `GET /api/migrate/repo-vs-profile` as state-changing because its handler commits a backfill. Other GET handlers retain read continuity even when they reconcile caches or derived local facts.
- Treat every POST other than the explicit query/dry-run shapes as state-changing, including `/api/language-server` and unknown future routes. Treat PUT, PATCH, and DELETE as state-changing by default. Future mutating GET/HEAD routes are forbidden unless their exceptional semantics are added to both the classifier and route contract; the current manual route inventory is review evidence, not the safety mechanism for unknown non-read methods.
- Stamp all direct same-origin worker requests from `tonk-ui`, `tonk-host`'s site registration and sync keepalive, and trusted portal relays of normalized, explicitly allowed `/api` data-plane paths (including durable blob upload and LSP POST). Deny account/profile controls, repository lifecycle, global site/sync, inspection, cross-reach paths, and unknown routes before fetch. Strip guest-supplied internal provenance from provider/control/public paths. Do not stamp account/access-provider requests, `/.well-known/tonk`, `/ucan/`, or deployment artifact probes: those are network/service surfaces, not worker `/api` routes.
- Reuse `tonk_host::bridge::context_headers()` as the browser header source so `x-tonk-build`, site, path, and hash behavior cannot drift between host and UI clients. Native compilation and tests supply no browser headers unless a focused request-construction test injects them.
- Reuse only existing workspace crates: `tonk-host` depends on the existing `tonk-worker-api` wire crate so request and response header constants cannot drift; retain that required Cargo.lock package edge and add no external dependency. Run Cargo with `CARGO_INCREMENTAL=0` and stop broad compilation if filesystem free space falls below 15 GB.
- Preserve the existing update-ready prompt and its safe copy. Nested relays must reach that same top-document prompt without consuming or rewriting the response body. The related Hub security correction intentionally replaces the guest account roster with a labelled navigation to trusted Settings; Storybook documents that user-visible tradeoff.

## File map

- `rust/tonk-worker/src/router.rs`: classify request effects, parse strict build headers, and return typed stale/invalid-build responses.
- `rust/tonk-worker-api/src/lib.rs`: own the stale-build response marker shared by worker and UI.
- `rust/tonk-worker/src/router/route_table.rs`: pin current mutating and explicitly read-like route examples against the classifier.
- `rust/tonk-worker/src/router/evaluate.rs`: expose one strict predicate for the classifier's canonical, unambiguous dry-run query.
- `rust/tonk-ui/index.html`, `rust/tonk-ui/scripts/stamp-service-worker.sh`: emit and synchronously publish immutable document provenance; keep the live version probe discovery-only.
- `rust/tonk-ui/src/worker_client.rs`: own worker readiness, same-origin `/api` URL construction, context/build headers, and stale-response update notification.
- `rust/tonk-ui/src/lib.rs`: register the internal worker client module.
- `rust/tonk-ui/src/api.rs`: route every direct local-worker request through the adapter while leaving external provider requests on raw `reqwest`.
- `rust/tonk-ui/src/register_dialog.rs`: route invite-status query polling through the same adapter.
- `rust/tonk-host/src/bridge.rs`, `rust/tonk-portal/src/bridge.rs`: propagate immutable build provenance into nested sealed guests and let the trusted relay replace it only on normalized worker paths.
- `rust/tonk-host/src/http.rs`: expose one crate-local context-header applicator, stamp site registration, and observe the exact stale response marker before any caller handles the body.
- `rust/tonk-host/src/host.rs`: stamp the state-changing sync keepalive through the shared header source and observe its response marker.
- `rust/tonk-worker/src/cache.rs`, `rust/tonk-worker/src/worker.rs`: retain older generation caches at activation.
- `rust/tonk-ui/assets/service_worker.js`: non-destructive withdrawal and failure-page recovery.
- `rust/tonk-workspace/src/ui_hub_account.rs`, `rust/tonk-workspace/src/ui_hub_account.html`: neutral trusted-Settings handoff with no guest roster/control calls.
- `docs/storybook/ui/routing-and-runtime.md`: document the stale-page read/write contract and recovery behavior.
- `docs/storybook/app/data.json`, `docs/storybook/app/data.js`: regenerate the product-map artifacts.

### Task 1: Make the worker's stale-build decision fail closed for writes

**Files:**

- Modify: `rust/tonk-worker/src/router.rs:request effect and stale-build middleware`
- Modify: `rust/tonk-worker/src/router/evaluate.rs:dry-run query predicate`
- Modify: `rust/tonk-worker/src/router/route_table.rs:route effect contract tests`
- Test: `rust/tonk-worker/src/router.rs:handshake_tests`

**Interfaces:**

- Consumes: an Axum request method, path/query, optional current worker build, and zero or more `x-tonk-build` header values.
- Produces: `RequestEffect::{ReadOnly, StateChanging}` and either pass-through, structured `409 stale-build`, or structured `400 invalid-build-header` for a classified write.

- [x] Add `it_classifies_every_declared_state_changing_route` with concrete method/path examples for every current mutating route; run `CARGO_INCREMENTAL=0 cargo test -p tonk-worker it_classifies_every_declared_state_changing_route -- --exact`; expect failures for every route outside `/transact` and `/evaluate`.
- [x] Add focused cases for unknown POST/PUT/PATCH/DELETE, exact profile/repository query POSTs, near-miss query paths, unambiguous dry-run evaluate values, duplicate/malformed dry-run values, and the mutating migration GET. Run the focused classifier tests; expect the current suffix logic to misclassify them.
- [x] Implement a small request-effect classifier whose only POST read exceptions are exact route shapes and the shared dry-run predicate. Keep all unknown non-read methods state-changing.
- [x] Add strict header/middleware tests: matching write passes; mismatched write returns typed 409; mismatched read passes; missing header passes; present empty/non-text/duplicate header returns typed 400 before the handler. Observe RED against the permissive `HeaderMap::get` logic, then implement the strict parser and response.
- [x] Run the focused handshake and route-table tests with nonzero counts, then the relevant native `tonk-worker` library tests.

### Task 2: Put every direct UI worker request behind one stamped adapter

**Files:**

- Create: `rust/tonk-ui/src/worker_client.rs`
- Modify: `rust/tonk-ui/src/lib.rs:module declarations`
- Modify: `rust/tonk-ui/src/api.rs:local /api request construction`
- Modify: `rust/tonk-ui/src/register_dialog.rs:invite-status query`
- Test: `rust/tonk-ui/src/worker_client.rs:tests`

**Interfaces:**

- Consumes: an HTTP method and an origin-relative path beginning with `/api`; on wasm, context headers from `tonk_host::bridge::context_headers()`.
- Produces: a ready-gated worker request with an absolute same-origin URL and each context header set exactly once; rejects non-worker paths before network IO and raises the existing update-ready event only for a marked stale-build response.

- [x] Add request-construction tests covering every direct UI mutation path (identity root; evaluation/transact/sync/join; account attach/name/custody/device/activation/profile switch/add/unlink/deletion), plus a read/query and missing-build case. Run the exact tests; expect RED because no adapter exists and raw `reqwest` requests have no `x-tonk-build`.
- [x] Implement the adapter with a private injected-context constructor for native tests and a single wasm production source from `tonk-host`. Do not introduce a second build-id reader.
- [x] Move all local `/api` calls in `api.rs` and `register_dialog.rs` through the adapter. Leave external account/access service and deployment requests unchanged.
- [x] Run the focused adapter tests and a native `tonk-ui` test filter with nonzero counts. Audit remaining direct `reqwest` calls to confirm none target `/api` outside browser-test helpers.

### Task 3: Close the two host-owned mutation bypasses

**Files:**

- Modify: `rust/tonk-host/src/http.rs:append_context_headers and post_site_to`
- Modify: `rust/tonk-host/src/host.rs:spawn_keepalive`
- Test: focused wasm/native source-contract tests following the existing host test conventions

**Interfaces:**

- Consumes: the same `context_headers()` list used by the UI adapter.
- Produces: site-registration and `/api/sync?why=keepalive` requests carrying the page build when one exists, while preserving missing-header sealed/native compatibility.

- [x] Add focused tests that construct site/keepalive request headers; the site test observed the missing `x-tonk-build` RED and the keepalive source audit found the same unstamped bypass.
- [x] Expose one fallible context-header builder inside `tonk-host`, use it for site registration, overwrite only the caller-authoritative `x-tonk-path`, and use the same builder for keepalive.
- [x] Run the focused host tests and the Wasm compile check; retain site routing and subscription continuity.

### Task 4: Document and verify the complete contract

**Files:**

- Modify: `docs/storybook/ui/routing-and-runtime.md:service-worker update behavior`
- Modify: `docs/storybook/app/data.json`
- Modify: `docs/storybook/app/data.js`
- Verify: all files above

**Interfaces:**

- Consumes: the existing `UI-03` service-worker/update recovery behavior.
- Produces: product-map evidence that stale pages retain reads/subscriptions but receive an actionable refusal before writes; no new visual state or destructive recovery.

- [x] Update the Storybook source with the exact header compatibility, read exceptions, typed stale/invalid refusal, and “reload to update” recovery. Record the dry-run and migration assumptions.
- [x] Run `python3 docs/storybook/scripts/build.py`, `python3 docs/storybook/scripts/build.py --check`, and `python3 docs/storybook/scripts/check-links.py docs/storybook`; expect regenerated source/data parity and all links green.
- [x] Run `CARGO_INCREMENTAL=0 cargo fmt --all -- --check`, the focused native worker/UI/host tests with nonzero counts, `CARGO_INCREMENTAL=0 cargo check -p tonk-ui --target wasm32-unknown-unknown`, relevant focused wasm tests with nonzero counts, all Node service-worker tests, and `git diff --check`.
- [x] Confirm no cache/storage/registration clearing, external dependency,
  provider request stamping, or unrelated UI change. The only lockfile change
  records the existing internal `tonk-host -> tonk-worker-api` wire dependency.
  Commit one coherent local change without pushing.

### Task 5: Close independent-review transport and provenance gaps

- [x] Add an artifact test proving the post-build step emits one identical build id in `index.html`, `service_worker.js`, and `version.json`; observe RED when the document had no immutable metadata, then stamp it in the existing post-build transaction.
- [x] Add a boot contract proving immutable metadata is published before the Rust loader and the live version probe cannot replace it; observe RED against the asynchronous `/version.json` assignment, then make the probe discovery-only.
- [x] Add a source contract covering the real blob-upload and LSP POST paths; observe RED while sealed guests had no build and the relay trusted caller headers, then propagate the build and make the trusted relay normalize the URL, delete caller provenance, and set one host value only for `/api`.
- [x] Add a host source contract proving JSON, site, SSE, asserted-notation, and keepalive responses use one exact header observer before body handling; observe RED against the body-substring/ignored-response paths, then centralize them.
- [x] Run the focused browser-Wasm behavioral tests for host marker timing/body preservation and portal build overwrite/provider exclusion once the shared disk floor permits the repository wrapper.
- [x] Re-run focused/broader Node, Rust, Wasm, Storybook, formatting, and diff checks; record exact counts below.

### Task 6: Close guest authorization and non-destructive recovery blockers

- [x] Add RED regressions for direct and dot-segment-normalized guest access to account/profile controls, repository lifecycle, global/inspection routes, forged internal headers, and cross-repository reach; retain positive query/transact/evaluate/blob/LSP and public-asset cases.
- [x] Normalize the guest URL once, authorize with an explicit method/path allowlist and portal reach before stamping/fetch, and default-deny everything else. Replace every guest-supplied internal header with trusted host values only on authorized worker requests.
- [x] Remove Hub roster loading, profile activation/add, active-profile presentation, and guest-triggered credential creation. Make **account** a labelled navigation to trusted `/settings`; keep profile listing/switch/add and ceremony confirmation in the top document.
- [x] Add RED regressions showing activation cannot purge older generation caches, then remove automatic generation cleanup so retained clients/workers keep their sole offline copies.
- [x] Add RED nested-host regressions for the exact stale marker, then relay the signal through each portal/host layer to the trusted top prompt without consuming the response body.
- [x] Add RED withdrawal/boot contracts requiring immutable page-generation comparison and no unregister/cache deletion/client navigation. Make matching withdrawal a latched data-plane refusal with update/reload guidance; make repeated failure-page recovery update/reload/retry only.
- [x] Add RED artifact evidence that outer service-worker policy changes the build id. Validate complete outputs before publication, exclude overlapping stampers, and restore the prior artifact set after catchable failures.
- [x] Update the durable Hub plan, READMEs, Storybook feature/verification source, WEB-02 source inventory, and generated product map. Record that exact in-Hub switching is intentionally replaced by trusted Settings until top-document account chrome exists.
- [x] Run and record the final focused/broad Node, native/Wasm, browser integration, Storybook, formatting, clippy, and diff evidence after the last code/doc change. Record the disk-guarded browser command and unrelated baseline clippy findings rather than treating either as product evidence.

## Execution record

### Assumptions and deliberate boundaries

- Production build stamps are the 16 lowercase hexadecimal identifiers emitted by `stamp-service-worker.sh` into the worker and immutable document metadata. Development and genuinely pre-protocol contexts retain the missing-header compatibility path.
- Missing build metadata remains compatible even for a write. This is an explicit rollout tradeoff for older pages, not evidence that the builds match: an old page can still mutate through a newer worker by omitting the header. The header is compatibility provenance, not authentication or a security boundary. Any present invalid or ambiguous metadata fails closed.
- Query/subscription POSTs are read-like only at the two exact route shapes. Evaluate is read-like only with one decoded, canonical lowercase `transact=false`; aliases accepted by the handler are conservatively write-gated.
- Existing GET/HEAD routes remain overlap-compatible except the committing repository/profile migration. Some ordinary GET handlers perform current-worker-owned idempotent reconciliation (status/outbox refresh, lazy mount, or view binding); they do not interpret a stale page body. A future GET/HEAD whose page input authorizes a mutation must be added to the exceptional classifier and route contract. A direct browser navigation cannot carry this custom header, so the migration's primary visitable form still uses the documented missing-header compatibility path.
- External account/access-provider calls, `/ucan/`, and `/.well-known/tonk` are not worker requests and remain on their existing clients without Tonk worker headers.
- No new visual state was added. A marked stale-build response dispatches the existing update-ready event; an invalid-header response does not prompt, reload, clear data, or alter registrations.
- Built-in Hub components are not trusted principals because they execute in the same sealed realm as arbitrary authored scripts. Therefore guest account/profile controls are denied even though that removes the prior in-Hub roster UX; trusted Settings is the explicit temporary handoff.
- No generation cache is purged automatically. This can retain unused cache generations until browser storage pressure evicts them; the tradeoff is accepted because liveness/reference tracking is not yet sufficient to prove an old client or worker no longer needs its only offline artifact.
- Withdrawal is not treated as authenticated destructive authority. A matching same-origin flag may stop the exact immutable generation from serving further work, but it cannot delete caches/storage, unregister a worker, or navigate clients.
- The stamp script is catchable-failure atomic, not power-loss atomic. It locks, validates, backs up, and rolls back ordinary/signal failures. `SIGKILL`, host power loss, and independent live publication of the three files still require deployment-level directory staging and atomic promotion; a retained lock/backups after rollback failure require operator inspection.

### TDD evidence

- Route inventory RED: the old suffix classifier failed first at `DELETE /api/account` (`0 passed; 1 failed`). Read-exception RED: canonical `evaluate?transact=false` was still classified as a write (`0 passed; 1 failed`).
- Strict-header RED: an empty present header was misreported as stale instead of invalid (`0 passed; 1 failed`). Typed-marker RED: the stale `409` had no response marker (`0 passed; 1 failed`).
- UI adapter RED: the direct profile transaction carried no `x-tonk-build` (`0 passed; 1 failed`). The update-notifier test then failed to compile with missing `notify_on_stale_build` before the central send wrapper existed.
- Host RED: site registration produced no `x-tonk-build` (`0 passed; 1 failed`); the source audit found the same omission in the sync keepalive.
- Guest-boundary RED: raw account/profile and repository-control fetches reached the host, normalized dot segments could escape the inspected reach, and guest-authored internal headers survived request construction. The new raw-port regressions failed before the relay was normalized, classified, and stamped at one chokepoint.
- Hub-boundary RED: the component exposed roster loading, activation/add controls, and guest-triggered credential creation. The replacement contract initially failed the checked-in standard-library CSS/markup expectations until those expectations were changed to the trusted Settings handoff.
- Retention/recovery RED: activation deleted other generation caches; withdrawal and repeated-failure recovery deleted caches or registrations; the document revocation check compared against mutable `version.json`. The final live-withdrawal regression additionally observed `0` worker update releases where `1` was required before new work was refused.
- Nested-signal RED: a marked response inside a sealed guest dispatched only an iframe-local DOM event. The host and real `MessageChannel` regressions failed until the exact signal could relay through each portal layer.
- Artifact RED: changing only outer service-worker policy left the build id unchanged, and an injected publication failure could leave the three stamped files from different generations. The publisher now hashes canonicalized policy and restores every prior output after catchable failures.

### Final evidence and residual gap

- Native: worker handshake `7 passed`, route table/effect contract `4 passed`, UI worker client `3 passed`, standard-library contracts `16 passed`, portal library `4 passed`, and workspace library `16 passed`.
- Wasm/browser: the immutable browser archive ran all selected host/portal/workspace tests (`166 passed`, `0 failed`), including raw guest control denial, dot-segment normalization, forged-header replacement, positive same-reach data/LSP behavior, nested stale propagation, and the static Hub handoff. The focused top-level browser integration for the Hub-to-Settings navigation also passed (`1 passed`, `0 failed`) through the real Nix/Caddy/WebDriver seam in `933.20s`. The locked four-package Wasm check passed after the final Rust production changes.
- Node: all service-worker/artifact source contracts passed (`54 passed`, `0 failed`), including cache retention, immutable withdrawal comparison, non-destructive recovery, live worker shutdown, outer-policy provenance, and catchable stamp rollback. Storybook regenerated and checked at `26` screens, `78` journeys, `115` verification items, and `6` triage findings; all `173` local links passed.
- Formatting and source sanity: `cargo fmt --all -- --check`, `git diff --check`, and `sh -n rust/tonk-ui/scripts/stamp-service-worker.sh` passed after the final code change.
- Relevant host/portal/workspace clippy passed with `-D warnings`. Broader clippy remains blocked by unrelated existing lints in `tonk-display/src/element.rs`, `tonk-worker/src/router.rs`, and `tonk-ui` (document-list formatting, `needless_return`, dead helpers, type complexity, and an obfuscated conditional); no new lint from this change was reported.
- A second live browser regression for explicit Settings confirmation was interrupted, not failed, after Nix unexpectedly started another broad derivation and free disk fell from `41 GB` to about `19 GB`; the planned multi-account browser run was not started. The archived component/bridge suite and the first real browser handoff remain the production-seam evidence. The branch-local reproducible cleanup target is this worktree's `target/` (`16 GB` at the stop point); it was not deleted.
- Source audit: the production `tonk-ui` raw-client `/api` bypass grep returned no matches (exit `1`); the direct host keepalive calls the shared header builder and response observer. The trusted portal now stamps normalized worker requests made by sealed components, including blob upload and LSP, while stripping the header from provider/control paths.
- The real two-generation `UI-03` browser checklist and a full live multi-account Settings-switch run remain open. Layer tests prove classification, stamping, response signaling, browser-target behavior, and the trusted handoff, but do not simulate an old deployed page, nested sealed guest, and newly activated worker end to end. Pre-protocol pages remain deliberately unclassifiable until a later enforcement rollout can distinguish them safely.
