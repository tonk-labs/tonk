# Service-worker stale-write barrier implementation plan

**Goal:** Prevent a protocol-capable page or sealed guest from mutating Tonk through a service worker from another build while preserving reads, subscriptions, dry-run evaluation, and an explicit migration path for genuinely older contexts that cannot send a build stamp.

**Approach:** Derive immutable document build metadata from the outer service-worker policy, worker glue/Wasm, and the final deployed full-digest browser resource graph; stamp it into `index.html` before the app can mount, and keep live `version.json` strictly as update discovery. Install verifies the manifest and every UI/lazy/guest response before populating an isolated incoming generation, while every retained cache is read-only and eviction misses fail closed. Route only exact stamped members through that cache; leave live edge routes and registered guest requests to Rust/network, and redirect slashless static-site navigations to their exact stamped directory route. Put top-document UI requests to the local `/api` worker surface behind one build-aware request adapter. The trusted portal normalizes each guest URL once, default-denies undeclared/control routes, enforces the portal's repository reach, strips caller provenance, and stamps only explicit worker data-plane requests. Its author-facing `/api/language-server` alias is resolved after authorization to an exact repository/branch endpoint using the same canonical identity-segment codec as LSP URIs, with worker sessions and outbound diagnostics partitioned by trusted scope and client. Nested bridges retain a bootstrap-captured relay capability before authored code runs, so trusted descendant provenance never traverses authored `window.fetch`. Replace the worker's suffix-only write check with a default-safe method-and-route policy: explicit read-like POST routes pass, known state-changing exceptions are pinned, and every other non-read method is treated as a write. Exact stale markers relay through nested hosts to the existing trusted top prompt. Activation retains generation caches, withdrawal/failure recovery never deletes local artifacts or unregisters workers, and every automatic reload and global client claim waits for both the current page predicate and an origin-global durable account-setup hold to prove safety.

**Constraints:**

- Preserve local IndexedDB, CacheStorage, profiles, passkeys, and registrations. This change must not add any automatic clear, unregister, generation purge, or destructive reset; the existing guarded update-alignment/watchdog reload rules remain the only automatic reloads.
- Install must use only build-produced provenance: fetch `asset-manifest.json`, worker Wasm, and every listed shell/UI/lazy/guest asset with `no-store`, verify full SHA-256 asset digests before the first incoming-cache write, and reject the install on any missing or mismatched response. Retained `TONK_SHELL_*` and `TONK_WORKER_*` caches are never repaired, backfilled, or purged; an eviction miss returns an actionable failure, except worker Wasm may be used ephemerally after its stamped digest re-verifies.
- Mutable deployment controls (`version.json`, `kill-switch.json`) and generated stamp outputs are outside the immutable resource graph. In a stamped production worker, authored `no-store`, `reload`, and `no-cache` flags cannot escape the sealed graph; only the unstamped development worker honors them for hot reload.
- Repository, profile, and branch identities use one reversible canonical segment codec for portal LSP endpoints, worker route validation, LSP URI roots, and server URI parsing. Reserved bytes are uppercase percent-encoded (`feat/artifact` → `feat%2Fartifact`); raw, lowercase, and over-encoded aliases fail closed.
- Every automatic alignment, update, watchdog-recovery, and development hot-swap reload must consult `document.documentElement[data-tonk-account-setup-critical]`, fail closed if `window.tonkAccountSetupMayReload()` throws or does not return exactly `true`, and resume from `tonk:account-setup-critical-change`. Before an Arm, account setup must also durably publish the agreed origin-global IndexedDB hold under the shared exclusive Web Lock. Both the page and service worker treat missing Web Locks, malformed/unreadable storage, or any live hold as unsafe; successor claim and the irreversible reload callback execute under that same lock, and a clear advisory BroadcastChannel signal causes a fresh authoritative read.
- Preserve an existing stale page's GET/HEAD reads, query POSTs/SSE subscriptions, and an evaluate request only when its parsed query contains exactly one lowercase `transact=false` value. The handler's looser `0`/`no`/case-insensitive aliases remain valid requests but are conservatively write-gated across a build mismatch.
- Treat missing `x-tonk-build` as unclassified and compatible for genuinely pre-protocol or development pages. Current generated pages and their sealed guests carry immutable provenance. Treat a present empty, non-text, duplicate, or otherwise invalid build header as a typed fail-closed error on a classified write.
- Treat `GET /api/migrate/repo-vs-profile` as state-changing because its handler commits a backfill. Other GET handlers retain read continuity even when they reconcile caches or derived local facts.
- Treat every POST other than the explicit query/dry-run shapes as state-changing, including scoped `.../language-server` routes and unknown future routes. Treat PUT, PATCH, and DELETE as state-changing by default. Future mutating GET/HEAD routes are forbidden unless their exceptional semantics are added to both the classifier and route contract; the current manual route inventory is review evidence, not the safety mechanism for unknown non-read methods.
- Stamp all direct same-origin worker requests from `tonk-ui`, `tonk-host`'s site registration and sync keepalive, and trusted portal relays of normalized, explicitly allowed `/api` data-plane paths (including durable blob upload and LSP POST). Deny account/profile controls, repository lifecycle, global site/sync, inspection, cross-reach paths, and unknown routes before fetch. Strip guest-supplied internal provenance from provider/control/public paths. Do not stamp account/access-provider requests, `/.well-known/tonk`, `/ucan/`, or deployment artifact probes: those are network/service surfaces, not worker `/api` routes.
- Reuse `tonk_host::bridge::context_headers()` as the browser header source so `x-tonk-build`, site, path, and hash behavior cannot drift between host and UI clients. Native compilation and tests supply no browser headers unless a focused request-construction test injects them.
- Reuse only existing workspace crates: `tonk-host` depends on the existing `tonk-worker-api` wire crate so request and response header constants cannot drift; retain that required Cargo.lock package edge and add no external dependency. Run Cargo with `CARGO_INCREMENTAL=0` and stop broad compilation if filesystem free space falls below 15 GB.
- Preserve the existing update-ready prompt and its safe copy. Nested relays must reach that same top-document prompt without consuming or rewriting the response body. The related Hub security correction intentionally replaces the guest account roster with a labelled navigation to trusted Settings; Storybook documents that user-visible tradeoff.

## File map

- `rust/tonk-worker/src/router.rs`: classify request effects, parse strict build headers, and return typed stale/invalid-build responses.
- `rust/tonk-worker-api/src/lib.rs`, `rust/tonk-worker-api/src/lsp_scope.rs`: own the stale-build response marker, trusted LSP client header, and canonical LSP authority codec shared across boundaries.
- `rust/tonk-worker/src/router/route_table.rs`: pin current mutating and explicitly read-like route examples against the classifier.
- `rust/tonk-worker/src/router/evaluate.rs`: expose one strict predicate for the classifier's canonical, unambiguous dry-run query.
- `rust/tonk-ui/index.html`, `rust/tonk-ui/assets/hot-swap.js`, `rust/tonk-ui/scripts/stamp-service-worker.sh`: emit and synchronously publish immutable document provenance, keep the live version probe discovery-only, and gate every automatic reload on durable account setup.
- `rust/tonk-ui/src/worker_client.rs`: own worker readiness, same-origin `/api` URL construction, context/build headers, and stale-response update notification.
- `rust/tonk-ui/src/lib.rs`: register the internal worker client module.
- `rust/tonk-ui/src/api.rs`: route every direct local-worker request through the adapter while leaving external provider requests on raw `reqwest`.
- `rust/tonk-ui/src/register_dialog.rs`: route invite-status query polling through the same adapter.
- `rust/tonk-host/src/bridge.rs`, `rust/tonk-portal/src/bridge.rs`: propagate immutable build provenance into nested sealed guests and let the trusted relay replace it only on normalized worker paths.
- `rust/tonk-host/src/http.rs`: expose one crate-local context-header applicator, stamp site registration, and observe the exact stale response marker before any caller handles the body.
- `rust/tonk-host/src/host.rs`: stamp the state-changing sync keepalive through the shared header source and observe its response marker.
- `rust/tonk-worker/src/cache.rs`, `rust/tonk-worker/src/worker.rs`, `rust/tonk-ui/assets/service_worker.js`: retain older generation caches and make runtime reads immutable after install; keep withdrawal and failure-page recovery non-destructive.
- `rust/tonk-worker/src/router/lsp.rs`, `rust/tonk-worker/src/router/lsp_env.rs`: expose only repository/branch-scoped LSP endpoints, validate every accepted message shape, enforce scope again before opening live data, and isolate/filter each client's outbound stream.
- `rust/tonk-language-server/src/server.rs`, `rust/tonk-inspector/src/element.rs`: parse and emit canonical scoped buffer URIs, including legal slash-bearing branch identities.
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

### Task 7: Seal retained generations and scope the language server

- [x] Add RED shipped-source regressions showing normal navigation, a waiting-successor navigation, and cached static assets could delete or overwrite entries in the outgoing generation. Remove runtime revalidation/backfill: install is the sole generation-cache writer.
- [x] Publish a deterministic `asset-manifest.json` containing the exact full-SHA-256 shell/UI/lazy/guest graph and bind it into `BUILD_ID`. Fetch and verify the manifest, every asset, and worker Wasm before opening the incoming caches; publish/rollback the manifest with the worker, document, and version metadata.
- [x] Fail retained shell/static misses coherently without accepting live stable-name bytes online or offline. On worker-Wasm eviction, fetch with `no-store`, re-verify, and boot ephemerally without writing `TONK_WORKER_*`.
- [x] Add RED worker regressions for cross-repository, cross-branch, and cross-profile document/workspace URIs; unknown or ambiguous JSON-RPC shapes; and diagnostics crossing either repository or client boundaries.
- [x] Replace the worker-global LSP route and server/broadcast state with exact named/profile repository + branch routes and per-scope/client sessions. Default-deny unknown inbound and outbound methods, validate every nested URI-bearing field, and re-enforce scope in `LspEnvProvider` before opening live data.
- [x] Keep `/api/language-server` only as a portal authoring alias. Resolve it from the portal's trusted reach, canonicalize the worker target, replace any authored client header, and deny direct scoped paths outside that reach before fetch.
- [x] Share one canonical identity-segment codec across portal endpoint generation/authorization, worker route and URI roots, and language-server parsing. Add positive `feat/artifact` and negative cross-scope/alias contracts.
- [x] Re-run the focused Node/static, targeted worker/API/language-server/portal native evidence, and the portal production-Wasm compile permitted by the serialized build slot under the 15 GB disk floor. Update the runtime/verification docs and generated Storybook data, then commit without pushing; stop further build expansion once free space reaches the floor.

### Task 8: Close post-review lifecycle, crash recovery, nested-client, and artifact gaps

- [x] Add a real service-worker lifecycle RED proving an installing candidate
  that becomes `redundant` must not call the incumbent's irreversible
  `onupdatefound` hook. Observe the existing eager teardown, then defer it until
  `installed` (or a durable `waiting` successor at startup) and pin exactly-once
  retirement after success.
- [x] Replace same-build final-cache inference with nonce-named staging caches
  and one durable generation marker whose `building`, `publishing`, and
  `adopted` states define the only legal recovery mutations. Add crash-retry
  coverage at both staging and final publication, adopted/incomplete retention,
  marker eviction, and stable names with no adoption provenance.
- [x] Extend portal LSP principals across nested relays with a bounded canonical
  chain of host-minted random segments. Strip caller authority at every relay,
  prevent a descendant from replacing an ancestor, reject duplicate/non-canonical
  worker headers, and cover same-scope multi-hop sibling isolation.
- [x] Add a deterministic source fingerprint to the checked-in `tonk-code`
  bundle and an executable production-artifact regression that drives a real
  `503 {"control":"update-pending"}` response through the bundled provider,
  observes the reconnect hold, and resumes only on `controllerchange`.
- [x] Make browser-asset enumeration a checked producer step rather than a
  `find | sort | while` pipeline whose upstream failure could publish a
  truncated manifest; pin an injected partial-output failure.
- [x] Regenerate and review the complete `tonk-code/assets` output graph once
  the serialized Node build slot and disk floor permit it, then make both the
  executable artifact behavior and source-freshness regressions green.
- [x] Run focused native/Wasm portal, worker/API, service-worker Node, artifact,
  Storybook source/generated parity, formatting, shell syntax, and diff checks
  after the last generated artifact change. Record any build-slot or browser
  validation left unexecuted rather than inferring it from source tests.

### Task 9: Close release-gate routing, publication, private-capability, and account-safety gaps

- [x] Stamp the exact immutable path set into both service-worker layers. Serve
  only exact top-level members from the sealed shell cache, delegate live edge
  and registered nested-client requests to Rust/network, keep ordinary SPA
  navigations on `/`, and redirect slashless static-site paths to their exact
  stamped trailing-slash documents before relative assets resolve.
- [x] Restamp the completed Cloudflare browser tree after guide and Storybook
  overlays. Emit both physical `*/index.html` members and directory aliases so
  the final `BUILD_ID`, manifest, install verification, and routing policy cover
  every controlled browser asset.
- [x] Capture the sealed guest relay before authored markup executes and pass
  the retained function directly into Wasm through a non-exported Rust seam.
  Make nested portal fetches use that capability so authored `window.fetch`
  wrappers cannot observe or replay trusted descendant-principal headers.
- [x] Add the strict origin-global account hold reader and claim gate to both
  the page and service worker using IndexedDB `tonk-update-safety-v1`, the
  exclusive Web Lock of the same name, and advisory BroadcastChannel wakeups.
  Re-read inside the lock and invoke claim/reload handoff before releasing it;
  treat malformed storage, read/open failure, and missing Locks as held.
- [x] Add `tsconfig.json` to the checked-in `tonk-code` source fingerprint and
  regenerate the tracked production bundle.
- [x] Update the runtime READMEs and Storybook source/generated data, then run
  focused Node/source/static, formatting, shell-syntax, link, and diff gates.
  Do not infer Cargo/Wasm/live-browser behavior from those checks.

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
- A generation is complete and immutable after install. Its build-published manifest binds the shell, UI, lazy, and guest resources by full digest; an eviction miss returns an actionable `503` online and offline because live stable-name bytes cannot prove membership in the retained build. Worker Wasm is the narrow exception: matching stamped bytes may boot the current instance after eviction, but are never written back.
- Mutable deployment controls are not generation assets. `version.json`, `kill-switch.json`, stamp outputs, and temporary/backup/lock files remain outside the manifest. A stamped production generation ignores caller-authored cache-bypass flags for static resources; only an unstamped development worker uses them for hot reload.
- Account-update safety has two scopes. The DOM attribute/predicate remains authoritative for the current page, while IndexedDB `tonk-update-safety-v1` store `holds`, key `account-setup`, carries the origin-global hold `{version:1, kind:"account-setup", operationId:<64 lowercase hex>, leasedRevision:<canonical u64 decimal>}`. Absence is the only globally safe value; malformed/unreadable state and missing Web Locks fail closed. The page and service worker re-read under the same exclusive lock, and the irreversible claim/reload handoff runs before releasing it so another tab cannot publish an Arm hold in a check-to-action gap. BroadcastChannel messages are advisory wakeups only.
- `/api/language-server` remains an author-facing URL only inside a portal with one trusted `with` reach. The portal resolves it to an exact repository/profile + branch endpoint and replaces client identity after authorization. There is intentionally no worker-global endpoint and no ambiguous `allow`-only choice; a trusted top-document client must select the deep scoped route and is isolated by its service-worker client id.
- Withdrawal is not treated as authenticated destructive authority. A matching same-origin flag may stop the exact immutable generation from serving further work, but it cannot delete caches/storage, unregister a worker, or navigate clients.
- The stamp script is catchable-failure atomic, not power-loss atomic. It locks, validates, backs up, and rolls back ordinary/signal failures. `SIGKILL`, host power loss, and independent live publication of the four files still require deployment-level directory staging and atomic promotion; a retained lock/backups after rollback failure require operator inspection.
- A successor reaching `installed` is the chosen irreversible-retirement boundary. Merely entering `installing` is not sufficient; `redundant` before install leaves the incumbent live. Once install succeeds, the browser owns whether the waiting worker activates, and the incumbent remains terminally retired as designed.
- The generation marker's adopted response is the cache commit point. Browser eviction of that marker while stable caches survive makes the generation unverifiable and therefore unavailable; retry fails closed without deleting, opening, backfilling, or otherwise guessing the provenance of those stable names.
- A nested canonical LSP value is descendant naming input, not direct authority: every relay prepends its own 128-bit host-minted segment, while malformed or duplicate input collapses to that relay alone. The sealed runtime captures the trusted relay function before authored content runs and passes it directly into Wasm; nested portal traffic therefore does not traverse an authored `window.fetch` wrapper that could observe or replay a legitimate child segment. The retained function is a capability, not an author-visible header value.
- `tonk-code` freshness covers package metadata, the locked dependency graph, build/fingerprint scripts, `tsconfig.json`, and every TypeScript source input. Generated language entries are excluded by directory and regenerated before each fingerprint; the committed stable bundle is the only tracked artifact changed because the remaining split graph retained identical content hashes.
- The initial `tonk-ui` stamp is not the deployment identity. The Cloudflare derivation overlays guide and Storybook and then restamps that completed tree; every static-site `index.html` has both a physical path and trailing-slash route in the manifest. Only exact stamped paths use immutable caching. `/.well-known/tonk`, other unmanifested edge paths, and nested-client requests remain live Rust/network routes.

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
- Generation-seal RED: the shipped worker still scheduled old-shell revalidation, deleted old hashed assets, overwrote `/`, and cached live-deployment static misses into `TONK_SHELL_<old>`. The focused Node contracts failed until navigation and static cache hits became read-only and every miss stopped short of a cache write.
- LSP-scope RED: the new cross-repository/client tests did not compile because there was no `LspScope` or scoped session key, and the only production endpoint shared one server and broadcast channel globally. The final boundary accepts only explicit same-scope message shapes, denies foreign/unknown/ambiguous input, checks the environment open, and filters each per-client outbound stream.
- Complete-install RED: the real install hook produced only a stub root and no UI/guest/lazy graph; a mismatched response could be discovered only after cache state existed. The publisher and install contracts now prove the full graph before any incoming-cache write, and the positive offline test reaches guest and worker-Wasm bytes populated only by the production install hook.
- Eviction RED: worker-Wasm recovery performed a runtime `put` into `TONK_WORKER_<old>`, static misses booted Rust and accepted live bytes, and Rust's cache adapter fetched a live miss. Focused Node contracts failed on the observed mutation and live fallbacks before all retained-generation miss paths became read-only/fail-closed.
- Retained-verification RED: checking an evicted old generation with `caches.open()` recreated both retained cache names. Completeness verification now uses named `CacheStorage.match()` reads only; the regression proves neither old name reappears.
- Stable-name escape RED: an authored production `cache: "no-store"` request reached the live network under an old stamped controller. Both JS and Rust now honor cache bypasses only for `BUILD_ID === "dev"`; production serves the sealed response or the coherent miss.
- Mutable-control RED: the first exact graph assertion found `/kill-switch.json` in the generated manifest. The publisher now excludes that live withdrawal control alongside `version.json` and generated stamp metadata.
- Account-reload RED: the alignment/watchdog tests reloaded during the top-document critical section, the update source contract found `claim` outside the reload gate, and the final real-script regression observed one automatic successor claim while critical when zero was required. Shared fail-closed durability gates now defer every automatic path; load alignment claims only after durability, while the update affordance executes claim immediately before its eventual safe reload.
- Canonical-scope RED: raw route/URI interpolation split the legal branch `feat/artifact`. The shared codec, portal, worker, language-server, and inspector contracts pin its `feat%2Fartifact` round trip and reject lowercase, raw, over-encoded, foreign-branch, and foreign-repository aliases; focused native execution is now green.
- Successor-lifecycle RED: the real service-worker VM observed one irreversible Rust `onupdatefound` retirement as soon as a candidate entered `installing`; after that candidate became `redundant`, the incumbent could not resume. The worker now observes candidate state and retires exactly once only after `installed`, while the failed-install path observes zero retirements.
- Interrupted-install RED: a retained partial same-build final cache was rejected with no recoverable ownership proof. Unique stage names plus the durable `building` / `publishing` / `adopted` marker now recover both pre-publication and partial-publication crashes, while markerless stable names and incomplete adopted finals remain untouched and fail closed.
- Publisher-enumeration RED: an injected `find` printed one asset and exited `75`, but the old pipeline still published a successful truncated graph. Enumeration and sorting are now standalone checked producers; the same injected failure exits nonzero and restores the prior outputs.
- Production-artifact RED: the checked-in `tonk-code.js` retried immediately after a real `503 {"control":"update-pending"}` and carried no current-source fingerprint. The regenerated bundle holds one stream until `controllerchange`, then opens exactly one successor stream, and its first-line SHA-256 matches the complete source input set.
- Nested-client RED: the outer portal replaced the inner portal's identity, collapsing same-scope nested siblings onto one worker session. The shared bounded chain codec, relay composition, and worker validation now preserve distinct complete descendants; malformed, non-canonical, over-depth, and duplicate direct values cannot alias them.

### Final evidence and residual gap

- Native: worker handshake `7 passed`, route table/effect contract `4 passed`, UI worker client `3 passed`, standard-library contracts `16 passed`, portal library `4 passed`, and workspace library `16 passed`.
- Wasm/browser: the immutable browser archive ran all selected host/portal/workspace tests (`166 passed`, `0 failed`), including raw guest control denial, dot-segment normalization, forged-header replacement, positive same-reach data/LSP behavior, nested stale propagation, and the static Hub handoff. The focused top-level browser integration for the Hub-to-Settings navigation also passed (`1 passed`, `0 failed`) through the real Nix/Caddy/WebDriver seam in `933.20s`. The locked four-package Wasm check passed for the preceding stack, before Task 7. After Task 7, `cargo check -p tonk-portal --target wasm32-unknown-unknown` passed with `CARGO_INCREMENTAL=0`. The inspector's browser-only URI assertion compiled in the native crate but did not execute; no further Wasm/browser build was started after free space reached the 15 GB floor.
- Node: all service-worker/artifact source contracts passed (`73 passed`, `0 failed` across `14` suites), including complete install, mutable-control exclusion, retained-cache immutability, production cache-bypass containment, account-critical reload/claim deferral across production and hot-swap paths, immutable withdrawal comparison, non-destructive recovery, live worker shutdown, scoped portal-to-worker LSP routing, outer-policy provenance, and catchable stamp rollback. Storybook regenerated and checked at `26` screens, `78` journeys, `115` verification items, and `6` triage findings; regeneration produced no derived-data diff and all `173` local links passed.
- Task 7 native: `tonk-worker-api` codec tests passed (`2 passed`, `0 failed`); the focused language-server URI/scope tests passed (`3 passed`, `0 failed` across two commands); `cargo test -p tonk-worker --lib router::lsp::tests` passed (`11 passed`, `0 failed`, `99` filtered); and portal library tests passed (`4 passed`, `0 failed`). These cover canonical slash-bearing identities and alias rejection, same-scope message use, cross-repository/branch/profile rejection, unknown and ambiguous input, initialize-root rewriting, outbound filtering, two-client/scope isolation, terminal shutdown, and both exact deep route families.
- Formatting and source sanity: `cargo fmt --all -- --check`, `git diff --check`, and `sh -n rust/tonk-ui/scripts/stamp-service-worker.sh` passed after the final code change.
- Relevant host/portal/workspace clippy passed with `-D warnings`. Broader clippy remains blocked by unrelated existing lints in `tonk-display/src/element.rs`, `tonk-worker/src/router.rs`, and `tonk-ui` (document-list formatting, `needless_return`, dead helpers, type complexity, and an obfuscated conditional); no new lint from this change was reported.
- A second live browser regression for explicit Settings confirmation was interrupted, not failed, after Nix unexpectedly started another broad derivation and free disk fell from `41 GB` to about `19 GB`; the planned multi-account browser run was not started. The archived component/bridge suite and the first real browser handoff remain the production-seam evidence. The branch-local reproducible cleanup target is this worktree's `target/` (`16 GB` at the stop point); it was not deleted.
- Source audit: the production `tonk-ui` raw-client `/api` bypass grep returned no matches (exit `1`); the direct host keepalive calls the shared header builder and response observer. The trusted portal now stamps normalized worker requests made by sealed components, including blob upload and LSP, while stripping the header from provider/control paths. Focused greps found no runtime shell-cache delete/put/revalidation path, no worker-global `/api/language-server` route, and no relay fetch using the pre-authorization path.
- The real two-generation `UI-03` browser checklist and a full live multi-account Settings-switch run remain open. Layer tests prove classification, stamping, response signaling, browser-target behavior, and the trusted handoff, but do not simulate an old deployed page, nested sealed guest, and newly activated worker end to end. Pre-protocol pages remain deliberately unclassifiable until a later enforcement rollout can distinguish them safely.
- Task 8 focused evidence: worker-API chain tests passed (`2 passed`, `0 failed`); worker LSP tests passed (`12 passed`, `0 failed`, `99` filtered); and the portal production path passed `cargo check -p tonk-portal --target wasm32-unknown-unknown` after one RED type mismatch at the `Window`/`JsValue` reflection boundary was corrected. The real portal nested-relay browser test was not executed because this remediation turn explicitly prohibited Chrome and broader Wasm runners.
- Task 8 Node evidence: the production service-worker lifecycle/cache suites passed (`54 passed`, `0 failed`) and the publisher/executable-artifact suite passed (`7 passed`, `0 failed`). The regenerated tracked artifact graph changed only `assets/tonk-code.js` (`13` inserted lines, `12` removed); no new split chunk remained. Storybook source/generated parity stayed green at `26` screens, `78` journeys, `115` verification items, and `6` triage findings, with all `173` local links valid.
