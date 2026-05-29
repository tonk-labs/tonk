# Dependency-graph analyzer

Replaces the two-phase `resolve` (async, env-threaded, mutates `Scope`)
→ `expand` (sync, reads `Scope`) pipeline with an explicit dependency
graph compiled in three phases. Retires the `analyze_local` no-op-waker
hack and the `Option<&Env>` plumbing threaded through every `prefetch_*`.

## Why

The current design has three rough edges:

- **`analyze_local` drives an async `resolve` with a fake `Waker`** and a
  `NeverEnv` stub, asserting the future is `Ready` on first poll. It works
  only because every `prefetch_*` short-circuits on `env: None` before its
  `.await`. Fragile: any new genuinely-async step in `resolve` silently
  breaks the macro at runtime (`unreachable!`).
- **`Option<&Env>` is threaded through ~10 `prefetch_*` methods** purely so
  the env-free path can opt out. The opt-out is the same `let Some(env) =
  env else { return Ok(()) }` early-return everywhere.
- **Resolution order is implicit.** `resolve` walks document order and
  relies on it for in-doc symbol visibility (anchor declared before use).
  `prefetch_references` is a second ad-hoc walk. Deduplication of external
  lookups is incidental (the canonical-key cache, not the analyzer).

A graph makes the dependency structure explicit: each reference is a node
declaring what it needs; in-doc edges resolve first; only nodes still
unresolved get batched into env lookups (one lookup per distinct external
need, dedup by construction).

## Phases

```
push(syntax) -> Graph            // pure, sync, env-free
Graph::resolve(env) -> Resolved  // async; in-doc edges first, then env batch
Resolved::build() -> Tree        // pure, sync — today's `expand`
```

- **push** — walk the AST once, emit one `Node` per reference or
  declaration, recording in-doc edges. No IO. Equivalent to the
  declaration-registration half of today's `resolve` plus the reference
  collection of `prefetch_references`, but producing data (nodes/edges)
  instead of mutating `Scope`.

- **resolve** — drain the graph. A node whose inputs are all satisfied by
  earlier nodes resolves locally (in-doc). Nodes still unsatisfied after
  the local drain are the external set; they batch into env lookups
  (concept-by-name, symbol, attribute-by-entity, rule). Builtins resolve
  in the local drain (no env). `env: &Env` is taken once here, not
  threaded. Env-free callers pass a `LocalOnly` resolver: any external
  node remaining after the local drain is a hard error
  (`UnknownConcept` / `UnknownBookmark`), preserving today's
  `analyze_local` semantics.

- **build** — the existing `expand` body, reading resolved node outputs
  instead of `Scope` tables. Every lowering stays terminal.

## Node kinds

Derived from what `Scope` indexes today:

| Node | Produces | In-doc source | External lookup |
|------|----------|---------------|-----------------|
| `Anchor{name}` | entity | body-digest of the head | — (always local) |
| `Variable{name}` | entity | body-digest of the head | — |
| `ConceptRef{name}` | `ConceptDefinition` | `in_doc_concepts`, builtins | branch concept-by-name |
| `AttributeRef{name}` | `AttributeDefinition` | `in_doc_attributes` | branch attribute-by-name |
| `AttributeByEntity{entity}` | `AttributeDefinition` | `in_doc_attributes_by_entity` | branch attribute-by-entity |
| `SymbolRef{name}` | entity | anchors/variables/named | branch named-entity |
| `RuleRef{entity}` | `Option<Rule>` | — | branch rule (retract only) |
| `ConceptDecl{index}` | `DeclaredApplication` | parsed body | — (body may need `AttributeRef` edges) |
| `AttributeDecl{index}` | `DeclaredApplication` | parsed body | — |

Edges: `ConceptDecl` → `AttributeRef`/`AttributeByEntity` (its `with:`
map), `SymbolRef` → `Anchor`/`Variable` (in-doc symbol resolution),
`RuleRef` is a leaf (retract reads the installed rule).

## Resolver trait

```rust
trait Resolve {
    async fn concept(&self, name: &str) -> Result<Option<ConceptDefinition>, ResolveError>;
    async fn attribute(&self, name: &str) -> ...;
    async fn attribute_by_entity(&self, e: &Entity) -> ...;
    async fn named_entity(&self, name: &str) -> ...;
    async fn rule(&self, e: &Entity) -> Result<Option<Rule>, RuleResolveError>;
}
```

- `BranchResolver<Env>` — wraps `Source` + `&Env`, the real lookups.
- `LocalOnly` — every method returns `Ok(None)`; the env-free macro path.
  No fake waker: the local drain is genuinely sync, and `resolve` only
  awaits the resolver for the external set, which `LocalOnly` answers
  without IO.

`NeverEnv` and the `analyze_local` waker block are deleted.

## Migration

1. Add `graph.rs` (Node, Graph, push) and `resolve2.rs` (Resolve trait,
   BranchResolver, LocalOnly, drain). Keep `expand` untouched, fed from a
   `Resolved` built off the graph instead of `Scope`.
2. Port `expand`'s `Scope` reads to graph-output reads.
3. Delete `resolve`, `prefetch_references`, `prefetch_*`, `NeverEnv`, the
   waker block. `Scope` shrinks to the in-doc declaration tables the push
   phase fills directly (or folds into the graph entirely).
4. `analyze` / `analyze_local` become `push` → `resolve(BranchResolver |
   LocalOnly)` → `build`.

Tests: every existing analyzer test must pass unchanged (behavior parity);
add graph-shape tests (push produces expected nodes/edges; local drain
resolves a self-contained doc with zero resolver calls).

## Out of scope

The push/resolve/build split is the structural win. Incremental
re-resolution (caching resolved nodes across edits for the LSP) is a
follow-up — the graph makes it possible but this PR does not implement it.
