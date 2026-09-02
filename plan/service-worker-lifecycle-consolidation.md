# Service-worker lifecycle consolidation implementation plan

**Goal:** Ship one coherent, non-destructive service-worker lifecycle in which
each document, worker, and immutable browser asset belongs to a verified build;
updates are discovered and adopted without mixing generations; retiring workers
cannot be re-pinned; stale pages cannot write through a newer worker; and an
account ceremony can defer global claim/reload until its recovery state is
durable.

**Approach:** PR #800 is already the bootstrap/adoption foundation in
`staging`. Create three fresh branches from the newly merged tree—lifecycle
core, generation protocol, and nested-runtime boundaries—and port reviewed
behavior and tests from PRs #816–#818 plus the local WIP without merging or
rewriting those source branches. Land the generic account-update-safety
consumer in lifecycle core, then rebase the account-setup UI producer after
both the lifecycle and account-recovery stacks have landed. Prove the result
with real two-generation, multi-tab browser scenarios before closing the
superseded PRs.

**Constraints:**

- Preserve IndexedDB, CacheStorage, profiles, passkeys, service-worker
  registrations, and offline-only data. Normal update, withdrawal, boot-failure,
  and silent-stall handling must not clear storage or unregister workers.
- Keep one module registration at `/service_worker.js`, scope `/`, and
  `updateViaCache: "none"`. Do not use a versioned registration URL or parallel
  registrations.
- A document either mounts behind a controller from its own verified generation
  or remains on its last coherent offline generation. It never mounts against
  an unverified replacement.
- A first install may claim the current document. A replacement must not claim
  pre-protocol pages globally; only an update-aware page may request claim, and
  claim plus its one alignment reload cross the same safety gate.
- `version.json` and `kill-switch.json` are mutable discovery/withdrawal
  controls. Neither defines immutable document identity or destructive
  authority. The build stamped into `index.html`, the worker policy/glue/Wasm,
  and the verified asset manifest defines identity.
- Install is the only writer of a production generation cache. Retained
  generation caches are immutable: no stale-while-revalidate, backfill, repair,
  or automatic purge.
- A present malformed or mismatched page build fails closed before a classified
  write. Missing build metadata remains a documented rollout compatibility path
  for genuinely pre-protocol pages; it is not proof of equality or an
  authorization boundary.
- Reads, query subscriptions, and exactly canonical `transact=false` evaluation
  remain available across page/worker skew. Unknown non-read methods and routes
  are write-classified by default.
- Do not ship user-selectable rollback until the IndexedDB/data-format
  compatibility policy is separately decided and tested. Retaining old code
  caches is not equivalent to proving data rollback safe.
- The final production generation is stamped after Cloudflare guide/Storybook
  overlays, not from the earlier Trunk-only tree.
- Browser/headless behavior, build output, and CI are separate evidence. Node
  source contracts or a Wasm compile do not substitute for the two-generation
  browser matrix.
- Treat PRs #816–#818, local `fix/sw-update-lifecycle`, and local
  `fix/audit-account-setup-ui` as read-only source material until replacement
  PRs are live. Do not stash, clean, reset, rebase, force-push, or commit their
  dirty work as part of extraction.
- Start every replacement branch from a freshly fetched parent. Do not merge an
  old service-worker branch or cherry-pick a mixed commit wholesale; port the
  relevant failing test first, then its smallest reviewed production change.

## Evidence snapshot — 2026-08-31

### Audit coverage

The external audit's two critical findings are correct on current `staging`:
the service-worker glue is install-pinned while `worker_bg.wasm` is fetched from
a stable live URL, and a retiring worker can accept a replacement LSP/query
stream. Its proposed work maps as follows:

| Audit proposal | Existing work | Consolidation decision |
| --- | --- | --- |
| Atomic worker glue/Wasm and per-build caches | PR #816 and later local WIP | Keep in lifecycle core; extend identity to the final asset graph and crash-safe staged publication. |
| Terminal retirement and held SSE reconnect | PR #816 and local `tonk-code` artifact work | Keep in lifecycle core; retire only after a successor reaches `installed`, not merely `installing`. |
| Reachable update discovery and prompt | PR #800 plus PR #816 | Keep eager load-time update in #800 and background visible/online/periodic discovery in lifecycle core. |
| Failure recovery and remote withdrawal | PRs #800, #816, and #817 conflict | Keep bounded terminal recovery and non-destructive data-plane withdrawal. Reject #817's unregister-all behavior. |
| Page/worker version handshake | PR #816 | Keep as a separate generation-protocol layer with exact request classification. |
| HTTP caching and opaque-origin CORS | PRs #816 and #818 | Keep `_headers`; fold #818's source-derived allow-header test into generation protocol. |
| Per-version shell cache | PR #816 | Keep immutable `TONK_SHELL_<build>` / `TONK_WORKER_<build>` caches without automatic deletion. |
| Old-Safari message and same-origin cache check | PRs #800 and #816 | Keep. Defer navigation preload; it is performance work, not lifecycle correctness. |

`plan/offline-support.md` has a stale premise (the worker now does cache the
shell) and proposes a mutable `current` pointer plus user rollback that conflicts
with the sealed-generation model. Preserve its offline-availability goal, but
supersede its update/rollback mechanics with this plan.

### PR and WIP assessment

- [PR #800](https://github.com/tonk-labs/tonk/pull/800) merged into `staging` as
  `de0cab9886c1519593482325558c81ecdc270abe`. It supplies eager
  register/update, strict pre-mount readiness, page-directed claim, one guarded
  alignment reload, offline fallback, final-Wasm stamping, and non-destructive
  terminal boot behavior. Treat those as inherited baseline contracts and do
  not recreate them in a replacement PR.
- [PR #816](https://github.com/tonk-labs/tonk/pull/816) contains most of the
  right behavior but is not one reviewable lifecycle change. After #800 merged,
  its live branch was rebased to `2dfaf08c5d02bdcc81af670ccba4ee460af4e068`
  directly over `staging`; its published CLI/lint checks still fail because the
  internal `tonk-host -> tonk-worker-api` lockfile edge is missing. Local
  `fix/sw-update-lifecycle` retains the later review work at `e062d7d` plus a
  22-file dirty Task 9 continuation. The live PR and local worktree are two
  source histories, not parents for the fresh stack.
- [PR #817](https://github.com/tonk-labs/tonk/pull/817) correctly removes a
  network probe from the boot critical path and ships an empty flag, but its
  unregister-all withdrawal semantics are superseded by the later
  non-destructive refusal design. It is conflicting and based on an old #816
  intermediate. Fold only the non-blocking probe/real-JSON parts, then close it
  as superseded.
- [PR #818](https://github.com/tonk-labs/tonk/pull/818) is a valid one-file
  CORS invariant: every request-side `x-tonk-*` header read by worker routes must
  appear in `Access-Control-Allow-Headers`; `x-tonk-client-id` remains
  response-only. Its web failure comes from the old stacked cache test context,
  not the CORS behavior. Port/fold it into the fresh generation-protocol layer.
- [PR #838](https://github.com/tonk-labs/tonk/pull/838) and local
  `fix/audit-account-setup-ui` define the producer side of update safety:
  pre-WebAuthn stale-worker refusal, a page critical predicate, and a durable
  origin-global hold. The lifecycle consumer and account producer currently
  depend on each other in prose. Break the cycle by landing the generic consumer
  first; rebase the account UI after both parent stacks merge; run their composed
  artifacts before deployment.

## Target stack

```text
staging (contains merged PR #800 at de0cab9)
└── fix/sw-lifecycle-core               fresh PR: atomic install, retirement, discovery, recovery
    └── fix/sw-generation-protocol      fresh PR: stale-write barrier, request stamping, CORS
        └── fix/sw-nested-runtime       fresh PR: portal provenance and scoped LSP isolation

account provider/recovery stack (#835 → #836 → #838)
└── account-setup UI, freshly rebased after both lines above
    └── composed two-worker/two-tab release gate
```

The three replacement branches merge sequentially, but each must remain
independently buildable and reviewable. Do not make lifecycle core depend on the
account UI producer: absence of a hold is the only safe idle value.

## Extraction rules

- Fetch `origin/staging` immediately before creating each branch and resolve the
  exact parent SHA in the new PR body.
- Read committed source with `git show <source-ref>:<path>` and dirty source with
  `git -C <source-worktree> diff -- <path>`. Never use the source worktree as a
  new branch base.
- Port each focused RED test before its production implementation. A test that
  is already green on the fresh parent is baseline coverage, not evidence that
  a source patch was ported correctly.
- Compare every replacement branch against its immediate parent with
  `git diff --stat`, `git diff --check`, and a path audit. If a file belongs to a
  later slice, leave it out even when it shared an old commit.
- Preserve source provenance in the new PR description by naming the old PR,
  source ref, and behavior extracted. Do not preserve old commit topology at the
  expense of a coherent new review boundary.
- Keep #816–#818 open and unchanged until every retained behavior has a live
  replacement commit and test. Then close them as superseded; do not force-push
  them into the new shape.

## File map

- `rust/tonk-ui/index.html`: early registration/update owner, immutable document
  build publication, update/withdrawal prompt, reload safety gate.
- `rust/tonk-ui/assets/service_worker.js`: install/activate/fetch lifecycle,
  immutable generation caches, retirement, withdrawal, global claim gate.
- `rust/tonk-ui/scripts/stamp-service-worker.sh`: atomic final-tree manifest,
  digest, build-id, document, worker, and discovery-file publisher.
- `rust/tonk-ui/scripts/hash-guest.sh`, `flake.nix`: invoke the publisher at the
  Trunk and final Cloudflare artifact boundaries.
- `rust/tonk-ui/assets/_headers`: no-store mutable entry/control files and
  immutable content-hashed assets.
- `rust/tonk-host/src/ready.rs`, `rust/tonk-ui/src/bin/ui.rs`: verify the strict
  pre-mount readiness inherited from merged #800; change only if a fresh
  lifecycle test exposes a regression.
- `rust/tonk-worker/src/cache.rs`, `rust/tonk-worker/src/worker.rs`: immutable
  cache policy, worker activation/retirement, CORS, fetch-event lifetime.
- `rust/tonk-worker/src/router/lsp.rs`,
  `rust/tonk-code/src-js/diagnostics-provider.ts`: terminal stream shutdown and
  reconnect hold.
- `rust/tonk-worker/src/router.rs`,
  `rust/tonk-worker/src/router/evaluate.rs`,
  `rust/tonk-worker/src/router/route_table.rs`: request-effect classifier and
  build-skew middleware.
- `rust/tonk-worker-api/src/lib.rs`: shared `x-tonk-build` and
  `x-tonk-error-kind: stale-build` wire constants.
- `rust/tonk-ui/src/worker_client.rs`, `rust/tonk-ui/src/api.rs`,
  `rust/tonk-ui/src/register_dialog.rs`: one stamped top-document worker client.
- `rust/tonk-host/src/bridge.rs`, `rust/tonk-host/src/http.rs`,
  `rust/tonk-host/src/host.rs`: immutable page provenance and host-owned worker
  calls.
- `rust/tonk-portal/src/bridge.rs`: normalized/default-deny guest relay, trusted
  provenance replacement, and private nested relay capability.
- `rust/tonk-worker-api/src/lsp_scope.rs`,
  `rust/tonk-worker/src/router/lsp_env.rs`,
  `rust/tonk-language-server/src/server.rs`,
  `rust/tonk-inspector/src/element.rs`: canonical scoped LSP identities.
- `rust/tonk-ui/src/account_setup.rs`: account UI producer for the shared update
  hold and page critical state.
- `rust/tonk-ui/src/service_worker_upgrade.rs`,
  `rust/tonk-ui/tests/*.test.mjs`: real-browser and Node lifecycle contracts.
- `docs/storybook/ui/routing-and-runtime.md`,
  `docs/storybook/cross-cutting/failure-and-recovery.md`,
  `docs/storybook/verification/cli-spaces-ui.md`: user-visible behavior and
  verification contract.
- `plan/offline-support.md`: retain offline goals and mark its update/rollback
  mechanics superseded.

### Task 1: Preserve source WIP and establish a fresh staging baseline

**Files:**

- Verify only: merged `staging` at or after
  `de0cab9886c1519593482325558c81ecdc270abe`
- Read only: live PR refs #816–#818
- Read only: local `fix/sw-update-lifecycle` and
  `fix/audit-account-setup-ui` worktrees
- Test: `rust/tonk-ui/tests/service-worker-claim.test.mjs`
- Test: `rust/tonk-ui/tests/boot-terminal.test.mjs`
- Test: `rust/tonk-ui/src/service_worker_upgrade.rs`

**Interfaces:**

- Consumes: merged #800 behavior from `origin/staging`, the live PR heads, and
  the two unchanged local WIP worktrees.
- Produces: clean branch `fix/sw-lifecycle-core` whose initial commit is exactly
  the fetched `origin/staging`, plus a recorded source inventory for extraction.

- [ ] Fetch `origin/staging` and confirm
  `git merge-base --is-ancestor de0cab9886c1519593482325558c81ecdc270abe origin/staging` succeeds. Record the resolved `origin/staging`, #816, #817, and
  #818 SHAs before porting any patch.
- [ ] Record `git status --short --branch` for this worktree and both source
  worktrees, plus `git diff --stat` for each dirty source. Do not mutate either
  source; the before/after statuses must remain byte-for-byte identical.
- [ ] Run `node --test rust/tonk-ui/tests/service-worker-claim.test.mjs rust/tonk-ui/tests/boot-terminal.test.mjs`; expect first-install claim, page-directed replacement claim, first-message-wins terminalization, and non-destructive silent-stall cases to pass.
- [ ] Run `nix develop path:. -c cargo test -p tonk-ui --features integration-tests service_worker_upgrade::tests -- --test-threads=1 --nocapture`; expect online replacement to reload exactly once before mount and offline warm load to retain its active controller and state.
- [ ] Confirm `fix/sw-lifecycle-core` does not already exist locally or remotely,
  then run
  `git worktree add /Users/jackdouglas/tonk/tonk/.wt/fix/sw-lifecycle-core -b fix/sw-lifecycle-core origin/staging`.
  Before the first port, require `git -C /Users/jackdouglas/tonk/tonk/.wt/fix/sw-lifecycle-core diff --stat` to be empty and its `git rev-parse HEAD` to equal the recorded staging SHA.
- [ ] Re-run the two source-worktree status commands and compare them with the
  recorded output; expect no source commit, index, or working-tree change.

### Task 2: Publish one atomic immutable generation and make retirement terminal

**Files:**

- Modify: `rust/tonk-ui/assets/service_worker.js:installGeneration`,
  `watchSuccessor`, `retire`, `serveNavigation`, `routeFetch`, `failurePage`
- Modify: `rust/tonk-ui/scripts/stamp-service-worker.sh`
- Modify: `rust/tonk-ui/scripts/hash-guest.sh`
- Modify: `flake.nix:tonk-ui and tonk-cloudflare-artifacts post-fixup`
- Modify: `rust/tonk-ui/assets/_headers`
- Modify: `rust/tonk-worker/src/cache.rs:is_cacheable and onactivate policy`
- Modify: `rust/tonk-worker/src/router/lsp.rs:LspHub::shutdown`
- Modify: `rust/tonk-code/src-js/diagnostics-provider.ts:reconnect policy`
- Test: `rust/tonk-ui/tests/service-worker.test.mjs`
- Test: `rust/tonk-ui/tests/build-artifacts.test.mjs`

**Interfaces:**

- Consumes: fresh `fix/sw-lifecycle-core` at the exact `origin/staging` SHA from
  Task 1; no commit or merge from #816–#818.
- Produces: one 16-lowercase-hex `BUILD_ID` derived from canonical worker policy,
  worker glue/Wasm, and the final full-SHA-256 asset graph.
- Produces: `TONK_SHELL_<build>`, `TONK_WORKER_<build>`, and a durable
  generation marker with `building | publishing | adopted` states plus
  nonce-owned staging caches.
- Produces: terminal retiring state after a successor reaches `installed` (or is
  already `waiting` at restart); every later stream-open returns
  `503 {"control":"update-pending"}` and closes.
- Produces: the generic claim/reload consumer for IndexedDB/Web Lock update
  safety. With no account producer installed, hold absence is safe and #800's
  behavior is unchanged.

- [ ] Add/retain RED Node cases for mismatched Wasm, mismatched manifest asset,
  truncated asset enumeration, interrupted staging/publication, marker eviction,
  immutable retained caches, and an `installing -> redundant` candidate that
  must not retire the incumbent.
- [ ] Implement verified all-or-nothing install. Do not open a final cache before
  every fetched response passes its digest; write durable ownership before stage
  caches; make the adopted marker the commit point; clean only provably
  unadopted caches after a recoverable interruption.
- [ ] Stamp the final Cloudflare browser tree after guide/Storybook overlays.
  Include physical `*/index.html` members and their trailing-slash route aliases;
  route only exact stamped members through the immutable cache. Leave edge paths
  and registered nested clients to Rust/network.
- [ ] Make `LspHub::shutdown` terminal and add a worker-level retiring latch.
  Delay irreversible retirement until the incoming worker is installed; catch
  up from `registration.waiting` after an incumbent restart.
- [ ] Make the checked-in diagnostics bundle hold a real update-pending response
  until `controllerchange`, then open exactly one successor stream. Pin bundle
  freshness to its complete source/configuration input set.
- [ ] Keep update discovery on load, visible/online transitions, and a bounded
  periodic timer. Keep `version.json` discovery-only. Present update/withdrawal
  through the static boot/update surface without automatic storage deletion,
  unregistration, or repeated reload.
- [ ] Port the synthetic hold RED cases from local Task 9, then add the generic
  reader to `index.html` and the claim gate to `service_worker.js`. Both use
  IndexedDB/Web Lock name `tonk-update-safety-v1`, treat absent as safe and
  malformed/unreadable/live holds or missing Locks as unsafe, and perform the
  irreversible claim/reload callback before releasing the exclusive lock.
- [ ] Run `node --test rust/tonk-ui/tests/service-worker.test.mjs rust/tonk-ui/tests/build-artifacts.test.mjs rust/tonk-ui/tests/boot-script.test.mjs rust/tonk-ui/tests/boot-terminal.test.mjs`; expect every lifecycle, artifact, failure, and withdrawal contract to pass.
- [ ] Run `CARGO_INCREMENTAL=0 cargo test -p tonk-worker --lib router::lsp::tests -- --nocapture` and the focused post-shutdown subscription test; expect all existing and post-retirement subscriptions to terminate.
- [ ] Run `nix --accept-flake-config build --no-link .#tonk-cloudflare-artifacts`;
  verify one build id and one manifest digest across the final
  `index.html`, `service_worker.js`, `version.json`, and manifest.

### Task 3: Refuse stale writes through one generation protocol

**Files:**

- Modify: `rust/tonk-worker-api/src/lib.rs`
- Modify: `rust/tonk-worker/src/router.rs:request effect and build middleware`
- Modify: `rust/tonk-worker/src/router/evaluate.rs:canonical dry-run predicate`
- Modify: `rust/tonk-worker/src/router/route_table.rs:effect contract`
- Create: `rust/tonk-ui/src/worker_client.rs`
- Modify: `rust/tonk-ui/src/api.rs`, `rust/tonk-ui/src/register_dialog.rs`
- Modify: `rust/tonk-host/src/bridge.rs`, `rust/tonk-host/src/http.rs`,
  `rust/tonk-host/src/host.rs`
- Modify: `rust/tonk-worker/src/worker.rs:CORS allow/expose headers`
- Test: worker handshake/classifier, UI adapter, host observer, and opaque-origin
  CORS tests in the files above.

**Interfaces:**

- Consumes: the reviewed head of fresh `fix/sw-lifecycle-core` and opens fresh
  `fix/sw-generation-protocol` from that exact SHA.
- Produces: request header `x-tonk-build` and response marker
  `x-tonk-error-kind: stale-build`.
- Produces: structured `409 stale-build` for a mismatched classified write and
  structured `400 invalid-build-header` for an invalid present header; neither
  response consumes or disguises its typed body.
- Produces: one source-derived `ALLOWED_REQUEST_HEADERS` contract containing all
  request-side `x-tonk-*` headers read by routes; `x-tonk-client-id` remains in
  expose headers only.

- [ ] Before porting this slice, record `git rev-parse fix/sw-lifecycle-core`
  and run
  `git worktree add /Users/jackdouglas/tonk/tonk/.wt/fix/sw-generation-protocol -b fix/sw-generation-protocol fix/sw-lifecycle-core`.
  Require the new worktree HEAD to equal the recorded core SHA and its initial
  diff to be empty.
- [ ] Add RED route inventory cases for every current state-changing route,
  unknown POST/PUT/PATCH/DELETE, the mutating migration GET, exact query POSTs,
  and exact canonical `evaluate?transact=false`.
- [ ] Implement default-safe `RequestEffect::{ReadOnly, StateChanging}`
  classification and strict single-value build parsing. Preserve missing-header
  rollout compatibility and document it explicitly.
- [ ] Add RED request-construction cases for every direct top-document `/api`
  mutation, site registration, keepalive, read/query, and missing-build context;
  move them through one `worker_client`/host header source.
- [ ] Observe the exact stale response marker before callers consume bodies and
  dispatch the existing update-ready signal without auto-reload.
- [ ] Fold #818's source-derived CORS test here. Add a real opaque-origin browser
  case proving `x-tonk-build` preflight succeeds and the request reaches the
  worker; source grep alone is not the final browser boundary.
- [ ] Update `Cargo.lock` only for the internal `tonk-host -> tonk-worker-api`
  edge, then run `cargo check --locked` before CI. This is the missing published
  #816 fix.
- [ ] Run the focused native worker/UI/host tests, then
  `nix develop path:. -c test:web:debug -E 'package(tonk-worker) | package(tonk-ui) | package(tonk-host)'`; expect the classifier, request builder, response marker, and browser header cases to pass.

### Task 4: Bound nested guest provenance and LSP sessions

**Files:**

- Modify: `rust/tonk-portal/src/bridge.rs`
- Modify: `rust/tonk-host/src/bridge.rs`
- Create/modify: `rust/tonk-worker-api/src/lsp_scope.rs`
- Modify: `rust/tonk-worker/src/router/lsp.rs`,
  `rust/tonk-worker/src/router/lsp_env.rs`
- Modify: `rust/tonk-language-server/src/server.rs`
- Modify: `rust/tonk-inspector/src/element.rs`
- Modify/regenerate: `rust/tonk-code/assets/tonk-code.js`
- Test: portal raw-request, nested relay, canonical scope, and LSP isolation
  suites.

**Interfaces:**

- Consumes: the reviewed head of fresh `fix/sw-generation-protocol` and opens
  fresh `fix/sw-nested-runtime` from that exact SHA.
- Consumes: immutable document build provenance and portal repository/profile/
  branch reach.
- Produces: a normalized default-deny guest relay that strips authored internal
  headers, stamps only authorized same-origin worker data-plane requests, and
  never stamps provider/control/cross-origin requests.
- Produces: canonical uppercase percent-encoded LSP identity segments and
  per-scope/per-client sessions; nested relays extend a bounded chain with
  host-minted random segments through a bootstrap-captured private capability.

- [ ] Before porting this slice, record
  `git rev-parse fix/sw-generation-protocol` and run
  `git worktree add /Users/jackdouglas/tonk/tonk/.wt/fix/sw-nested-runtime -b fix/sw-nested-runtime fix/sw-generation-protocol`.
  Require the new worktree HEAD to equal the recorded protocol SHA and its
  initial diff to be empty.
- [ ] Add RED cases for direct/dot-segment guest control paths, cross-reach
  routes, forged provenance, nested siblings collapsing to one client, authored
  `window.fetch` interception, raw/lowercase/over-encoded aliases, and diagnostics
  crossing repository/branch/profile/client boundaries.
- [ ] Normalize once, authorize before fetch, replace every caller-controlled
  internal header, and retain the trusted relay function before authored markup
  executes. Pass that function directly into Wasm rather than routing nested
  trusted traffic through authored `window.fetch`.
- [ ] Resolve author-facing `/api/language-server` only after portal
  authorization to an exact scoped worker endpoint. Re-enforce the scope before
  opening data and on every inbound/outbound URI-bearing message.
- [ ] Keep the Hub account-control removal in its own security/product PR unless
  it is strictly required by the relay authorization boundary. Do not hide that
  user-visible change inside lifecycle core.
- [ ] Run focused `tonk-worker-api`, `tonk-worker`, `tonk-language-server`,
  `tonk-portal`, and `tonk-inspector` tests plus
  `nix develop path:. -c test:web:debug -E 'package(tonk-portal)'`; expect
  canonical round trips, alias rejection, nested sibling isolation, and exact
  stale-signal propagation to pass.

### Task 5: Compose account setup with claim/reload safety

**Files:**

- Verify: `rust/tonk-ui/index.html:automatic reload safety owner`
- Verify: `rust/tonk-ui/assets/service_worker.js:claimClientsWhenAccountSetupSafe`
- Verify: `rust/tonk-ui/assets/hot-swap.js`
- Modify: `rust/tonk-ui/src/account_setup.rs:update-safety producer`
- Modify: `rust/tonk-ui/src/register_dialog.rs:stale-worker and recovery copy`
- Test: `rust/tonk-ui/tests/service-worker-claim.test.mjs`
- Test: `rust/tonk-ui/src/account_flow.rs`

**Interfaces:**

- Consumes: the generic update-safety reader/claim gate from Task 2 and the
  merged account provider/recovery stack.
- Produces: IndexedDB `tonk-update-safety-v1`, store `holds`, key
  `account-setup`, value
  `{version:1, kind:"account-setup", operationId:<64 lowercase hex>, leasedRevision:<canonical u64 decimal>}`.
- Produces: exclusive Web Lock `tonk-update-safety-v1`, advisory same-named
  `BroadcastChannel` message `{type:"account-setup-hold-changed", version:1}`,
  current-page attribute `data-tonk-account-setup-critical`, predicate
  `window.tonkAccountSetupMayReload()`, and event
  `tonk:account-setup-critical-change`.

- [ ] Verify the Task 2 consumer contract unchanged on the composed parent:
  hold absence is idle; malformed/unreadable storage, a live hold, or missing
  Web Locks is unsafe; the authoritative read and irreversible callback occur
  under the same exclusive lock.
- [ ] On the account UI branch, write the hold under the same lock before
  `Arm`/WebAuthn can begin. Clear only after recovery is durably staged or a
  proven terminal pre-Arm outcome; publish the channel/event as wakeups, never
  as authority.
- [ ] Keep protocol-v2 worker/provider capability checks before WebAuthn. An old
  worker, missing route, 404, timeout, malformed capability, or deployment drift
  must show reload/update guidance without creating a credential.
- [ ] Rebase the account UI after lifecycle and #835/#836/#838 land. Do not
  deploy either producer or consumer alone while the composed test is absent.
- [ ] Add RED/GREEN browser cases for: stale worker before WebAuthn; unreadable
  or future hold; reload before Arm; reload after durable Stage; sibling tab
  detects an update while another tab holds Arm; worker restart while held; hold
  settlement releasing exactly one queued claim/reload.
- [ ] Run the exact focused account browser tests with virtual authenticators,
  then the lifecycle Node claim suite and the account worker/UI Wasm filters.
  Expect no duplicate passkey, provider account/device, claim, or reload.

### Task 6: Prove the deployed two-generation contract and retire stale PRs

**Files:**

- Modify: `rust/tonk-ui/src/service_worker_upgrade.rs`
- Modify: final Storybook source/generated files and READMEs.
- Modify: `plan/offline-support.md` supersession note.
- PR maintenance: close #816–#818 only after all three replacement heads exist
  and their retained behaviors/checks are mapped.

**Interfaces:**

- Consumes: separately built production artifacts A and B with distinct build
  ids and a harness that can switch the served artifact without clearing the
  browser profile.
- Produces: runtime evidence for update ordering, offline retention, multi-tab
  safety, withdrawal, and Safari behavior.

- [ ] Extend the mutable-worker harness to serve complete A/B artifact trees,
  not only rewrite a comment marker. Never use `unregister()`, site-data clear,
  CacheStorage deletion, or a new browser profile between phases.
- [ ] Add `it_adopts_a_complete_second_generation_without_mixing_assets`:
  hold A page/Wasm/guest requests, publish B, request update, and require one
  coherent A or B graph—never old glue/new Wasm or old worker/new shell.
- [ ] Add `it_releases_a_waiting_successor_after_old_streams_try_to_reconnect`:
  open query and LSP SSE, install B, attempt both reconnects against retiring A,
  and require immediate update-pending closes followed by B activation.
- [ ] Add `it_keeps_sibling_tabs_on_their_controller_until_claim_is_safe`:
  exercise old page/new worker, new page/old worker, a sleeping incumbent that
  missed `updatefound`, and the account hold from Task 5.
- [ ] Add `it_withdraws_a_generation_without_deleting_local_state`: publish a
  matching kill switch; require terminal/refused new work plus update guidance,
  with registrations, generation caches, IndexedDB, and passkeys intact.
- [ ] Re-run offline warm-load, failed/incomplete install, opaque guest CORS,
  nested stale propagation, and fresh first-install cases in the same final tree.
- [ ] Run `nix develop path:. -c test:web:debug` and
  `nix develop path:. -c cargo test -p tonk-ui --features integration-tests -- --test-threads=1 --nocapture`; expect all suites to complete. Report an
  interrupted/filtered run separately.
- [ ] Run the A→B matrix in current Safari/WebKit as a release gate. Record
  registration `installing/waiting/active`, document/worker build ids,
  `/api/health`, stream control frames, reload count, and cache names. Do not
  infer Safari from Chrome.
- [ ] Regenerate Storybook data, run base-aware impact and link checks, and make
  `UI-03` cite the executable A/B cases.
- [ ] After all replacement PR heads are live and green, update their stack
  descriptions and close #816, #817, and #818 as superseded with links to the
  exact replacement PR/commit for every retained behavior. Do not retarget or
  force-push the old PRs. Leave the new PRs unmerged until explicitly requested.

## Completion gate

The lifecycle is complete only when all of the following are fresh on the final
composed tree:

- exact final-artifact identity and atomic-install checks;
- focused native and Wasm worker/UI/host/portal/LSP checks;
- full Node lifecycle/artifact suites;
- full serialized Tonk UI browser suite;
- explicit A→B two-generation and two-tab account-hold scenarios;
- current Safari/WebKit runtime evidence;
- Storybook generated-data, link, and base-aware impact checks;
- `cargo fmt --all -- --check`, Nix formatting, `cargo check --locked`, and
  `git diff --check`;
- no ordinary-flow cache deletion, worker unregistration, IndexedDB reset,
  passkey reset, or unreviewed user-facing Hub change.

Anything not executed remains an explicit residual gap; partial browser, build,
or source-contract evidence must not be reported as end-to-end completion.
