# Slim the host page bundle — decouple the relay from the datalog engine

## Goal

The outermost page (`ui` bin in `tonk-ui`) is a thin service-worker relay:
it installs the host IO listeners, registers `<tonk-site>`, calls
`api::init()`, and mounts one `<tonk-site>`. All real rendering happens in
sealed guests (the `tonk-guest` bundle). The page has **no reason** to link
the datalog query engine (`dialog-query`) or the repository engine
(`dialog-repository`), yet it currently does.

## Baseline size (measured)

Fresh `nix develop --command cargo build -p tonk-ui --bin ui --release --target
wasm32-unknown-unknown`:
- **`ui.wasm` raw = 13,896,588 bytes (13.25 MB)** ← the number to beat.
- trunk dist `ui-*_bg.wasm` ≈ 14.82 MB (prior build; raw cargo is the
  apples-to-apples baseline).

## Decisions

- Each mirrored type gets a **round-trip test** decoding a sample of the SW's
  real serialized output and re-encoding identically (catch wire drift at test
  time).
- Portal's `Query`/`Conclusion` mirrors live in the **same `tonk-worker-api`
  crate** — one engine-free wire-types home for the whole page.

## Leak map (measured)

`dialog-query` reaches the `ui` bin through the `tonk-ui` crate's direct deps.
Reverse tree (`cargo tree -p tonk-ui --target wasm32-unknown-unknown -e no-dev
-i dialog-query`) shows two roots:

1. **`tonk-worker → tonk-ui`** (dominant). `tonk-ui/src/api.rs` does
   `use tonk_worker::{12 DTOs}`. `tonk-worker` pulls `dialog-reactor`,
   `tonk-schema`, `tonk-analyzer`, `tonk-evaluator` — the whole engine.
   `tonk-worker` is a legitimate dep of the *`worker` bin* (the SW), but the
   *`ui` bin* only needs the DTOs, as serde decode targets.
2. **`tonk-portal → tonk-schema → tonk-ui`**. `tonk-portal` (which the page
   uses via `register_site()`) depends on `tonk-schema`, which pulls the engine.

Per-bin wasm dead-code elimination means: if the `ui` bin's call graph never
reaches engine code, the linker drops it — even though `tonk-worker` stays a
crate dependency for the `worker` bin. So the fix is to make the page's code
reference only engine-free types.

## Type-home findings (measured)

- `Did` — lives in `dialog-varsig` / `dialog-capability`; **engine-free**
  (`dialog-varsig` → `dialog-query` count = 0). Safe to use directly.
- `SiteAddress` — is `dialog_network::NetworkAddress` re-exported by
  `dialog-repository`. `dialog-network` is **engine-free** (count = 0). Source
  it from `dialog-network` directly, not `dialog-repository`.
- `Revision` — lives in `dialog-repository` (which pulls `dialog-query` for its
  *other* code). But `Revision` itself is a plain serde struct (`Did`,
  `TreeReference`, `HashSet`, `usize`). The page only carries/deserializes it,
  never computes with it. Options: a wire-local mirror, or source from a lighter
  crate if one exposes it.

The page's DTO usage is construction + carry only:
`RemoteConfiguration::new(ucan_address)`, `BranchConfiguration::default()
.upstream(...)`, and returning the response structs to callers who read a
handful of fields (`.space`). No engine behavior is invoked.

## Design

### 1. `tonk-worker-api` — engine-free wire DTO crate

A types-only crate holding the 12 wire DTOs (`RepositoryInfo`, `ProfileInfo`,
`QueryResponse`, `EvaluateResponse`, `SyncResponse`, `SyncStatusResponse`,
`IdentifyResponse`, `JoinRequest`, `JoinResponse`, `BranchConfiguration`,
`RemoteConfiguration`, `RepositoryConfiguration`, and their sub-types). Deps:
`serde`, `serde_json`, `dialog-varsig` (Did), `dialog-network` (SiteAddress),
`lsp-types`. **Must NOT depend on `dialog-repository`, `dialog-query`,
`tonk-schema`, or `dialog-reactor`.**

- `Revision`: define an engine-free representation here (mirror struct) OR pull
  from a lighter home if one exists. Verify the wire (JSON) shape is identical —
  the SW serializes the real `Revision`; the page must decode byte-compatibly.
- `tonk-worker` re-exports these via `pub use tonk_worker_api::*` so its own
  handler code (which builds the DTOs from engine values) is unchanged. The
  surviving backup crate at scratchpad/tonk-worker-api-backup is a starting
  point but depended on `dialog-repository` — must be re-pointed to
  `dialog-network` + engine-free `Revision`.

### 2. Point the page at `tonk-worker-api`

- `tonk-ui/src/api.rs`: `use tonk_worker::{...}` → `use tonk_worker_api::{...}`.
- `tonk-ui/Cargo.toml`: add `tonk-worker-api`; keep `tonk-worker` (the `worker`
  bin needs it).
- The `worker` bin (`bin/worker.rs`) keeps `use tonk_worker::TonkServiceWorker`.

### 3. Cut the `tonk-portal → tonk-schema` edge (measured)

`tonk-portal` uses exactly **two** types from `tonk-schema`, both purely as
serde DTOs (never invokes their engine-coupled methods):

- `tonk_schema::query::Query` — `tonk-portal/src/query.rs:12`, return type of
  `entity_query()`. Built via `serde_json::from_value`, serialized back to
  `JsValue`. Never calls `into_concept_query()`. Leaks because its fields are
  `dialog_query::{ConceptDescriptor, ConceptQuery, Parameters}`.
- `tonk_schema::conclusion::Conclusion` (re-exported from `tonk-core`) —
  `tonk-portal/src/bridge.rs:49`, `Vec<Conclusion>` decoded from a host frame
  and re-serialized. Never calls `Conclusion::project()`. Leaks because
  `tonk-core/src/conclusion.rs` imports `dialog_query::{Any, ConceptConclusion,
  Parameters, Term}`.

Fix: give `tonk-portal` engine-free serde equivalents of just these two — a
`Query` whose `terms`/`predicate` are `serde_json::Value`/`IndexMap`, and a
`Conclusion` with `this: String` + `fields: BTreeMap<String, Ipld>` (its field
types are already engine-free; only the `project()` constructor + the
`dialog_query` import weigh it down). Home them in an engine-free crate (the
same DTO crate, or a portal-owned dialog-free module) and drop
`tonk-portal`'s `tonk-schema` dep (Cargo.toml:23). Wire format unchanged.

### 4. `tonk-host` is clean (measured)

`tonk_host::install()` is on the page's path but `tonk-host` pulls **no**
engine crate — its only dialog dep is `dialog-common`. `cargo tree -i
dialog-query -p tonk-host` errors "did not match any packages". Not an edge.

### Both leaks, one shape

Every leak is the same: the page/portal need only the **serde shape** of a type
whose definition happens to live in an engine-coupled crate. The fix is an
engine-free DTO layer, not a behavioral change. Confirmed leak edges into the
`ui` bin: (1) `api.rs → tonk_worker` DTOs, (2) `tonk-portal → tonk-schema`
{Query, Conclusion}. No others via `tonk-host`.

## De-mirroring (2026-07-11): single definitions, not copies

Per feedback ("we should not duplicate types"), pushed past the Revision/
SiteAddress dedup to also collapse the SyncState and Conclusion mirrors:

- **SyncState / Comparison / classify** — moved the whole sync-classification
  module from `tonk-schema` (engine-linked crate) INTO `tonk-worker-api` (it
  only needs the now-light `Revision`). `tonk-schema` re-exports at
  `tonk_schema::sync::*`. All 10 classify tests moved with it and pass.
- **Conclusion / Frame** — moved the engine-free data types from `tonk-core`
  INTO `tonk-worker-api`. The engine-coupled `project()` constructor stays in
  `tonk-core` as a free `fn project(&ConceptConclusion, &Parameters) ->
  Conclusion`. `tonk-core` re-exports `{Conclusion, Frame}`; the 5
  `Conclusion::project(...)` call sites (reactor ×4, worker bridge ×1) became
  `project(...)`. Re-export chain tonk-core → tonk-schema → dialog-reactor keeps
  every `conclusion::Conclusion` / `Frame` import resolving unchanged.
- **Query / Predicate** — LEFT mirrored. Its fields are genuine engine types
  (`ConceptDescriptor`, `Parameters`); de-mirroring needs those out of the
  engine (a large dialog change), disproportionate. Guarded by a wire-compat
  test. This is the one honest remaining seam.

Net: the only duplicated wire type is `Query`. Revision/SiteAddress/SyncState/
Conclusion/Frame each have ONE definition.

## Design pivot v2 (2026-07-11): LIGHT engine-free homes, opaque address

Refinement after two wrong homes:
- `dialog-storage` is engine-free but HEAVY (brotli/rexie/wasm-streams/s3s) —
  wrong home for `Revision`.
- `dialog-network` (SiteAddress) is ALSO heavy (s3/ucan/fs transports, 27 heavy
  deps) — can't type the DTO address field with it.

Correct homes (both light: 0 heavy deps):
- **`Revision` + `TreeReference` → `dialog-capability`** (owns `Did`, deps only
  `dialog-common`; `TreeReference` wraps a bare `[u8;32]` to keep the wire form
  independent of any `Blake3Hash` wrapper). `dialog-repository` re-exports.
  Cycle-safe: capability is BELOW varsig/repository, and common (which capability
  deps) does not need Did. Whole dialog-db workspace compiles.
- **DTO `address` → opaque `serde_json::Value`** in `tonk-worker-api`. The real
  `SiteAddress` is an externally-tagged transport enum
  (`{"Ucan":{"endpoint":"…"}}`, verified by probe). The page only builds/forwards
  it via `RemoteConfiguration::ucan(url)` and never inspects it, so a `Value` is
  faithful (no re-typing → no drift) and pulls no transport crate. This also let
  `tonk-ui` DROP its `dialog-remote-ucan-s3` dep (heavy) entirely.

Result: `tonk-worker-api` deps are serde/serde_json/indexmap/ipld-core/lsp-types
+ `dialog-varsig` + `dialog-capability` — all light, 0 engine, 0 transport. It
uses the REAL `dialog_capability::Revision`, so Revision/SiteAddress are NOT
duplicated. Remaining mirrors: `SyncState` (a plain enum in tonk-schema that now
only needs the light Revision — de-mirrorable), `Query`/`Conclusion`
(engine-typed impls/fields — honest seam).

## Design pivot (2026-07-11): engine-free types upstream, not mirrors

Mirroring `Revision` in `tonk-worker-api` duplicated a type and forced
conversions in every `tonk-worker` handler that builds a DTO (the failed
agent run). Root fix instead: move the plain data types to engine-free homes
so ONE definition works everywhere.

- **dialog-db change** (local worktree `/Users/gozala/Projects/dialog-db/worker/pery`,
  branch `feat/engine-free-revision` off `feat/inductive-self-negation`):
  `Revision` + `TreeReference` moved from `dialog-repository` (engine-linked)
  to `dialog-storage` (engine-free, already owns `Blake3Hash` which
  `TreeReference` wraps, already deps `dialog-capability` for `Did`; ZERO new
  deps). `dialog-repository` re-exports them at the historical paths. No new
  crate (per feedback). Proven locally via a `[patch]` on the tonk
  `Cargo.toml` pointing every dialog crate at the worktree; swap to a git pin
  after the dialog PR merges.
- `SiteAddress` was already `dialog_network::NetworkAddress` (engine-free).
- `tonk-worker-api` now uses the REAL `dialog_storage::Revision` +
  `dialog_network::NetworkAddress` — no Revision mirror, no duplication.
- Remaining mirrors (`SyncState`, `Conclusion`, `Query`) are for types whose
  real homes (`tonk-schema`/`tonk-core`) are engine-welded. `SyncState` is a
  plain enum only trapped by its crate (a tonk-side split could remove that
  mirror too — TBD). `Conclusion`/`Query` have engine-typed impls/fields, so
  a mirror is the honest seam. Guarded by wire-compat tests in `tonk-worker`.

## Progress (2026-07-11)

- [x] `tonk-worker-api` engine-free: deps are serde/indexmap/ipld-core/lsp-types
  + `dialog-varsig` (Did) + `dialog-network` (SiteAddress). `Revision`/
  `TreeReference` mirrored in `revision.rs` (round-trip test passes). Wasm tree:
  **0 dialog-query**.
- [x] Portal `Query`/`Conclusion` mirrors added to `tonk-worker-api`
  (`query.rs`, `conclusion.rs`). `tonk-portal` points at them, dropped its
  `tonk-schema` dep. `tonk-portal` wasm tree: **0 dialog-query** (`cargo tree -i
  dialog-query` = nothing to print).
- [x] `tonk-ui/src/api.rs` imports from `tonk_worker_api`; `tonk-worker-api`
  added to `tonk-ui/Cargo.toml` (kept `tonk-worker` for the worker bin).
- [~] `tonk-worker` re-exports the DTOs from `tonk-worker-api` + byte-compat
  test (in progress).
- [ ] Payoff: `ui` bin wasm tree 0 dialog-query, rebuild + measure vs 13.25 MB.

## Payoff check

After each edge is cut:
`cargo tree -p tonk-ui --target wasm32-unknown-unknown -e no-dev | grep -c
dialog-query` must trend to **0**. Then rebuild the `ui` bin release wasm and
compare bytes against the baseline the size agent captures.

## Risks / notes

- Wire-compat: the mirrored `Revision` (and any other mirrored type) must
  serialize/deserialize identically to the engine type. Add a round-trip test in
  `tonk-worker-api` that decodes a sample of the SW's real serialized output.
- This does not change runtime behavior — the page already only talks to the SW
  over HTTP/JSON. It's a link-time decoupling.
- `tonk-worker` as a *crate* dep of `tonk-ui` stays (worker bin). The win comes
  entirely from the `ui` bin's call graph no longer reaching engine code.
