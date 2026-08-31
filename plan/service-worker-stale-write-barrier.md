# Service-worker stale-write barrier implementation plan

**Goal:** Prevent a page from mutating Tonk through a service worker from another build while preserving reads, subscriptions, dry-run evaluation, and compatibility for older or sealed contexts that cannot send a build stamp.

**Approach:** Put all top-document UI requests to the local `/api` worker surface behind one build-aware request adapter that reuses `tonk-host`'s request-context header source. Replace the worker's suffix-only write check with a default-safe method-and-route policy: explicit read-like POST routes pass, known state-changing exceptions are pinned, and every other non-read method is treated as a write. Keep the existing structured stale-build response and update prompt rather than introducing another UI state.

**Constraints:**

- Preserve local IndexedDB, CacheStorage, profiles, passkeys, and registrations. This change must not clear, unregister, or reload anything automatically.
- Preserve an existing stale page's GET/HEAD reads, query POSTs/SSE subscriptions, and an evaluate request only when its parsed query contains exactly one lowercase `transact=false` value. The handler's looser `0`/`no`/case-insensitive aliases remain valid requests but are conservatively write-gated across a build mismatch.
- Treat missing `x-tonk-build` as unclassified and compatible for pre-protocol pages and sealed guests. Treat a present empty, non-text, duplicate, or otherwise invalid build header as a typed fail-closed error on a classified write.
- Treat `GET /api/migrate/repo-vs-profile` as state-changing because its handler commits a backfill. Other GET handlers retain read continuity even when they reconcile caches or derived local facts.
- Treat every POST other than the explicit query/dry-run shapes as state-changing, including `/api/language-server` and unknown future routes. Treat PUT, PATCH, and DELETE as state-changing by default. Future mutating GET/HEAD routes are forbidden unless their exceptional semantics are added to both the classifier and route contract; the current manual route inventory is review evidence, not the safety mechanism for unknown non-read methods.
- Stamp all direct same-origin worker requests from `tonk-ui`, plus `tonk-host`'s site registration and sync keepalive. Do not stamp account/access-provider requests, `/.well-known/tonk`, `/ucan/`, or deployment artifact probes: those are network/service surfaces, not worker `/api` routes.
- Reuse `tonk_host::bridge::context_headers()` as the browser header source so `x-tonk-build`, site, path, and hash behavior cannot drift between host and UI clients. Native compilation and tests supply no browser headers unless a focused request-construction test injects them.
- Add no dependencies or lock-file changes. Run Cargo with `CARGO_INCREMENTAL=0` and stop broad compilation if filesystem free space falls below 15 GB.
- Preserve the existing update-ready prompt and its safe copy. No visual or interaction change is required; Storybook documents the stronger refusal contract and unchanged read continuity.

## File map

- `rust/tonk-worker/src/router.rs`: classify request effects, parse strict build headers, and return typed stale/invalid-build responses.
- `rust/tonk-worker-api/src/lib.rs`: own the stale-build response marker shared by worker and UI.
- `rust/tonk-worker/src/router/route_table.rs`: pin current mutating and explicitly read-like route examples against the classifier.
- `rust/tonk-worker/src/router/evaluate.rs`: expose one strict predicate for the classifier's canonical, unambiguous dry-run query.
- `rust/tonk-ui/src/worker_client.rs`: own worker readiness, same-origin `/api` URL construction, context/build headers, and stale-response update notification.
- `rust/tonk-ui/src/lib.rs`: register the internal worker client module.
- `rust/tonk-ui/src/api.rs`: route every direct local-worker request through the adapter while leaving external provider requests on raw `reqwest`.
- `rust/tonk-ui/src/register_dialog.rs`: route invite-status query polling through the same adapter.
- `rust/tonk-host/src/http.rs`: expose one crate-local context-header applicator and stamp site registration.
- `rust/tonk-host/src/host.rs`: stamp the state-changing sync keepalive through the shared header source.
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
- [x] Confirm no cache/storage/registration clearing, dependency/lock change, provider request stamping, or unrelated UI change. Commit one coherent local change without pushing.

## Execution record

### Assumptions and deliberate boundaries

- Production build stamps are the 16 lowercase hexadecimal identifiers emitted by `stamp-service-worker.sh`; development or sealed contexts with no stamp retain the missing-header compatibility path.
- Missing build metadata remains compatible even for a write. This is an explicit migration tradeoff for older pages, not evidence that the builds match. Any present invalid or ambiguous metadata fails closed.
- Query/subscription POSTs are read-like only at the two exact route shapes. Evaluate is read-like only with one decoded, canonical lowercase `transact=false`; aliases accepted by the handler are conservatively write-gated.
- Existing GET/HEAD routes are treated as reads except the committing repository/profile migration. A future mutating GET/HEAD must be added to the exceptional classifier and route contract.
- External account/access-provider calls, `/ucan/`, and `/.well-known/tonk` are not worker requests and remain on their existing clients without Tonk worker headers.
- No new visual state was added. A marked stale-build response dispatches the existing update-ready event; an invalid-header response does not prompt, reload, clear data, or alter registrations.

### TDD evidence

- Route inventory RED: the old suffix classifier failed first at `DELETE /api/account` (`0 passed; 1 failed`). Read-exception RED: canonical `evaluate?transact=false` was still classified as a write (`0 passed; 1 failed`).
- Strict-header RED: an empty present header was misreported as stale instead of invalid (`0 passed; 1 failed`). Typed-marker RED: the stale `409` had no response marker (`0 passed; 1 failed`).
- UI adapter RED: the direct profile transaction carried no `x-tonk-build` (`0 passed; 1 failed`). The update-notifier test then failed to compile with missing `notify_on_stale_build` before the central send wrapper existed.
- Host RED: site registration produced no `x-tonk-build` (`0 passed; 1 failed`); the source audit found the same omission in the sync keepalive.

### Final evidence and residual gap

- Native: worker handshake `7 passed`, route table/effect contract `4 passed`, UI worker client `3 passed`.
- Wasm/browser: host site/keepalive header tests `2 passed` (`51 filtered`); UI worker client tests `3 passed` (`44 filtered`); `cargo check -p tonk-ui --target wasm32-unknown-unknown` passed. All Cargo commands used `CARGO_INCREMENTAL=0`.
- Node: all service-worker source contracts passed (`45 passed`, `0 failed`). Storybook regenerated and checked at `26` screens, `78` journeys, `115` verification items, and `6` triage findings; all `173` local links passed.
- Source audit: the production `tonk-ui` raw-client `/api` bypass grep returned no matches (exit `1`); the only direct host `/api` fetch is keepalive and it calls the shared header builder first.
- The real two-generation `UI-03` browser checklist remains open. Layer tests prove classification, stamping, response signaling, and browser-target compilation, but do not simulate an old deployed page and a newly activated worker end to end.
