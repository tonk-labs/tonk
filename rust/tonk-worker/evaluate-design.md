# Evaluate route — design notes

`POST /api/repository/{repo}/branch/{branch}/evaluate` accepts an
asserted-notation document (YAML or JSON) containing any mix of
queries and mutations, and runs the unified analyze → query →
plan → commit pipeline in a single transaction. The handler is a
thin glue layer over `tonk-notation` and `tonk-schema`; see
`rust/tonk-schema/analysis-spec.md` for the full analysis design.

## Pipeline

```
body bytes
   │  Content-Type
   ▼
parse                 ──→  Parsed { syntax, diagnostics }
   │
   ▼
syntax (tonk_notation::Syntax)
   │
   ▼  open(branch)
   │
BranchResolver       ──→  analyze(syntax, resolver) ──→ Analysis
   │
   ├─ analysis.query (optional unified ConceptQuery)
   │   └─ branch.query().select(unified) ──→ Vec<Parameters>  (match frames)
   │
   ▼  for each match frame
   │
Planner::plan(application, variables ∪ frame) ──→ ApplicationPlan
   │
   ▼  per Statement::Assert / Retract
   │
branch.transaction()
       .assert(plan)        ┐
       .retract(raw_claim)  ┘  (one transaction; one before/after pair)
       .commit()
```

Parse errors return `400 Bad Request` carrying the joined
diagnostic messages. Analyzer errors return `400` with the
`AnalyzeError` message. Anything past analyze is `500` if dialog
itself fails, `400` if the planner rejects (e.g. unbound
variable that no source binds).

## Response shape

`EvaluateResponse` carries:

- `revision_before` / `revision_after` — branch revisions captured
  on either side of the transaction commit (one before/after pair
  per request).
- `matches: Vec<QueryMatchBlock>` — one block per source query
  expression in document order. Each block has the head's display
  label and a list of `QueryResult { this, fields }` rendered by
  projecting each match against the source `Application`'s
  parameters.
- `commits: CommitSummary { claims, entities }` — number of EAVs
  written/retracted, plus a map from declared name (`.bookmark` or
  `?variable`) to entity URI for every binding the document made.

## BranchResolver

Implements `tonk_schema::interpret::Resolver` over an open
`Branch`:

- **`resolve_concept(name)`** — `Named` query for an entity with
  `dialog.meta/name = <name>` then walks `dialog.concept.with/*`
  claims to reconstruct the `ConceptDescriptor`.
- **`resolve_attribute(name)`** — same `Named` lookup then
  `AnonymousAttribute` query for the 5-field record, then
  descriptor reconstruction.
- **`resolve_attribute_by_entity(entity)`** — skips the name
  lookup; used when a `concept!`'s `with:` value is a `the:…` URI
  literal.

All three go through the typed concept builders in
`tonk_schema::concept` so the LSP and any other consumer can
resolve the same names through the same path.

## Why analyze → plan → commit (split, not inlined)

Splitting analyze (sync apart from `Resolver` trait calls) and
plan (variable substitution against per-match binding frames)
keeps each concern testable in isolation:

- The parser produces `Syntax` from text; no branch needed.
- `analyze(syntax, resolver)` produces an `Analysis` — a pure
  shape capturing what the document means. The LSP can call this
  against a thin in-memory resolver to surface diagnostics
  without touching dialog.
- `Application::plan(bindings)` substitutes `Term::Variable` slots
  against a binding frame and produces an `ApplicationPlan` — the
  fully concrete shape ready for `tx.assert` / `tx.retract`. No
  I/O.
- The worker is the only piece that does branch I/O at commit
  time — it runs the unified query if `analysis.query` is set,
  loops over match frames planning + queueing statements, then
  commits once.

Same logic, fewer hidden async layers.

## Retraction shape

`Statement::Retract` semantics: query the branch for current
values of any blank fields, then dissociate each match. Bound
fields (`Term::Constant`) act as match anchors and are not
retracted. Per `analysis-spec.md` example 5b: `name: "Alice"`
anchors, `age: _` is the only field dissociated.

The worker's `resolve_retraction_targets` walks the plan's
predicate, runs an `AttributeQuery` per blank field, and
collects `RawClaim { the, of, is }` triples. Those land in the
same transaction as the assertions before commit.

URI-bound retraction (`person! did:key:zX: _`) is fully wired.
Bookmark-bound retraction (`person! alice: _`) is partially
wired — the analyzer emits a blank `this` term because the
sync code path can't query the branch to resolve
`dialog.meta/name = alice` to an entity. A follow-up could
thread an async resolver through the retraction-build path.

## Bookmark uniqueness — git tag semantics

`dialog.meta/name` has cardinality `one`, so asserting a name on
a *new* entity automatically retracts the prior name claim from
the *old* entity (cardinality-one's defining behavior).

Combined with body-content-derived entities for non-meta
bookmark heads:

```yaml
person! alice:
  age: 25
```

→ `Entity::of(&{age: 25})` → entity X. `dialog.meta/name = "alice"`
on X.

Re-running the same body → same X, same name claim, no-op.

Re-running with `age: 26` → `Entity::of(&{age: 26})` → entity Y.
Cardinality-one retracts the name claim from X and asserts it on
Y. `.alice` now resolves to Y. Same as `git tag -f alice <new commit>`.

This makes the "cleanup-on-rename" pass unnecessary — dialog's
cardinality-one rule does the work.

## Async-trait dance

`Resolver` uses `#[cfg_attr(not(target_arch = "wasm32"), async_trait)]`
and the `?Send` variant on wasm. axum's handler bound needs
`Send` on native, but the actual runtime is `wasm_bindgen_futures`
where things aren't `Send`. The cfg_attr split is what dialog
itself does for `Source` / `Provider` traits.

The analyzer's `Scope` (in-document name index) uses the same
target-conditional shim: `Mutex` on native (so async-trait
futures stay `Send`), `RefCell` on wasm. `cell_borrow` /
`cell_borrow_mut` / `cell_new` helpers paper over the
difference; the read methods drop the borrow before any `await`
to avoid holding a guard across yield points.
