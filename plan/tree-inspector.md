# Tree inspector — visualize and introspect the underlying tree

Status: proposed. See and navigate the search/prolly tree that backs a
branch, the way `dialog-diagnose` does in its TUI but in the browser,
against the live IndexedDB-backed tree, with byte/size introspection so we
can find where commit cost concentrates (the perf thread that started
this: a ~51-expression scaffold is ~23 nodes but a single node can be
~150 KB, and that large-node re-encode dominates the CPU — see
`@gozala/2026-06-12.md`).

Two deliverables: a framework-agnostic visualizer web component,
**`dialog-arboretum`**, that knows nothing about tonk; and a thin
`tonk-display` view that feeds it tree data and embeds it.

## The core insight: keys are the legible unit

Node and entry counts are vast, so "show every row" does not scale
visually. The way through is the **key**. The tree is ordered by a fixed
composite index key, and the key's component structure determines node
boundaries and clustering. Visualize keys and key-ranges — color-coded by
component — and a node becomes "the range of key-space this leaf covers,"
legible no matter how many entries it holds.

### The key layout (verified, dialog-artifacts/src/key.rs)

A key is a fixed 162-byte array, components in sort order:

```
[ Tag : 1B ][ Entity : 64B ][ Attribute : 64B ][ ValueType : 1B ][ ValueRef : 32B ]
```

The tag byte (`Key::tag()`) selects the index ordering (entity /
attribute / value) — the three orderings live in **one** tree,
distinguished by this byte, not as separate trees. `KeyView` exposes
component readers (`.entity()`, `.attribute()`, `.value_type()`,
`.value_reference()`), so the worker decodes any key into structured
components; the client never sees raw bytes. This maps onto the existing
colored index-key diagram (Tag · Entity · Attribute · ValueType), the
atomic visual.

## Decode logic to reuse (verified, dialog-diagnose / dialog-repository)

`dialog-diagnose` is a native-only TUI (`#![cfg(not(wasm32))]`), so its
rendering is not reusable, but its node-decode model is, and it is built
on `dialog-artifacts` types that compile to wasm:

- A node decodes to either `Branch { upper_bound, children: Vec<hash> }`
  or `Segment { entries: Vec<Entry<Key, Value>> }`.
- The walk: `storage.get(&hash)` → bytes → `Node::new(buf).body()?` →
  `ArchivedNodeBody::Index` (branch) or `Segment` (leaf).

The worker reaches this via the branch handle: `branch.revision().tree` is
the single root hash; `branch.archive()` → `RepositoryArchiveExt::index()`
bridges to the search tree's hash-addressed `ContentAddressedStorage`.
Storage is append-only (no gc) and `flush()` yields only the final
deduplicated node set, so the resident node set is the committed tree —
the counts we read are real.

## Exposure: tree predicates over the existing /query endpoint

Rather than a bespoke route, expose the tree as **query predicates** so the
inspector is a `tonk-display` view over a query, reusing the whole
host/cache/subscription/display pipeline. The query model already supports
this in shape; resolution happens in tonk-worker, with no dialog change.

### Why this fits (verified, dialog-query)

A query is `Premise → Proposition`, and a `Proposition` is already a union
— `Concept`, `Formula`, `Attribute`, `Constraint`. Modes already exist:
each parameter has a `Requirement` (`Required` / `Optional`); the planner
refuses a premise whose Required params are unbound (`estimate()` →
`None`). So a moded predicate ("given a node hash, yield its fields") is
native, not invented. And `extract_parameters` binds **only the terms the
caller names**, so an operator may declare many fields at no cost to a
query that asks for few.

Two constraints keep resolution in-repo: the wire `Query` carries only one
predicate (a `ConceptDescriptor`), and `Formula::compute` is sync + pure
(no block I/O). So tonk-worker **intercepts** tree predicates and walks the
tree itself, emitting `Conclusion`s in the concept shape — dialog's
evaluator is not involved, which is exactly why neither constraint bites.

### Wire shape — mirror formula serialization

dialog discriminates premise kinds by JSON type (`proposition.rs`): an
object under `assert` is a Concept, a string is a Formula
(`{ "assert": "math/sum", "where": { … } }`). The wire `Query`'s
`predicate` is a `ConceptDescriptor`, which always serializes as an
*object*. So make `predicate` a tagged union by JSON type — object =
concept, **string = named tree operator** — the faithful mirror:

```json
// concept (object) — unchanged:
{ "predicate": { "with": { … } }, "terms": { … } }

// tree operator (string id, params in terms):
{ "predicate": "tree/node", "terms": { "hash": "did:key:zNode…" } }
```

The worker peeks at `predicate`: a string routes to the arboretum
resolver, an object to the existing concept path. `terms` stay
`Parameters`, results stay `Conclusion`s — host, cache, subscriptions,
display untouched.

### Operators

Declared like a `concept!`: a `with:` map of named parameters, each with a
`description` and an `as:` type — the node's fields. The one extension a
moded operator needs is an `input:` marker on the entry-point param(s):
concept fields are queryable in any direction, but a tree operator must be
handed its entry point (there is no node-hash index to scan from).
`input:` = Required; supply it or the operator is non-viable.

Because we cannot join, each operator is **self-contained** — it carries
the full set of fields you would want about the thing it yields, rather
than expecting you to join back to `tree/node` for them. Extra fields are
free (only requested terms bind).

```yaml
formula!: &tree/node
  description: A node in the branch's index tree, addressed by hash.
  with:
    hash:
      description: Hash of the node, as a did:key entity.
      as: entity
      input:
    kind:
      description: '"branch" or "leaf".'
      as: text
    size:
      description: Byte size of the node's stored block.
      as: integer
    level:
      description: Height in the tree (0 = leaf).
      as: integer
    count:
      description: Number of children (branch) or entries (leaf).
      as: integer

formula!: &tree/child
  description: A child of a branch node. Carries the child's own node
    fields plus its position, so no join back to tree/node is needed.
  with:
    hash:
      description: The branch node, as a did:key entity.
      as: entity
      input:
    at:
      description: Position among siblings.
      as: integer
    child:
      description: The child node's hash.
      as: entity
    kind:
      description: The child's kind — "branch" or "leaf".
      as: text
    size:
      description: The child's byte size.
      as: integer
    bound:
      description: The child's upper-bound key.
      as: entity

formula!: &tree/entry
  description: An entry stored in a leaf node.
  with:
    hash:
      description: The leaf node, as a did:key entity.
      as: entity
      input:
    at:
      description: Position within the leaf.
      as: integer
    key:
      description: The entry's composite index key.
      as: entity
    value:
      description: The entry's decoded value (or size-only; see open qs).
      as: text

formula!: &tree/key
  description: Decompose a composite index key into its components.
    Pure — loads no blocks; could become a real dialog Formula later.
  with:
    key:
      description: A key, from tree/entry or a constant.
      as: entity
      input:
    tag:
      description: Index ordering this key belongs to.
      as: text
    entity:
      description: Entity component, resolved to a short did:key.
      as: entity
    attribute:
      description: Attribute component, resolved to domain/name.
      as: text
    value-type:
      description: Value data-type component.
      as: text
    value-ref:
      description: Value reference component.
      as: entity
```

The tree root is the entry point: `branch.revision().tree` gives the root
hash directly (no operator needed — the view starts there and feeds it to
`tree/node` / `tree/child`).

A walk chains operators, each output feeding the next `input:`:

```
tree/node  { hash: <root> }     -> ?kind          # branch?
tree/child { hash: <root> }     -> ?child, ?kind, ?size   # one row per child
tree/child { hash: ?child }     -> ?leaf          # descend (child is a branch)
tree/entry { hash: ?leaf }      -> ?key, ?value   # entries at the leaf
tree/key   { key: ?key }        -> ?entity, ?attribute    # decode each key
```

Today the wire carries one predicate per request, so the client (or view)
issues one request per level — `tree/child` per expand. Carrying a
multi-premise body is a later extension; the operator semantics hold.

## The visualizer: `dialog-arboretum`

Framework-agnostic custom element, no tonk dependency, fed by the operator
results. Three nested visuals, each reusable:

1. `<tonk-key>` — one key as the colored component bar (Tag · Entity ·
   Attribute · ValueType · ValueRef) with human-readable resolution
   beneath. The index-key diagram, parameterized.
2. **Node as a key-range** — start/end key bars (varying components
   highlighted) + a size bar. Turns an opaque block into a labeled span.
3. **Tree outline** — `wa-tree` hosts the collapsible branch → child →
   leaf structure; each row is a node's range-bar + size-bar; expanding a
   leaf lists entries as `<tonk-key>`s. Lazy: a row loads children via
   `tree/child` on first expand.

Size bars run through every row so the byte-distribution half is always
visible. Optional later: a D3 treemap (node area = bytes) for a pure
size-weighted view.

## The embed: a `tonk-display` view

Resolves the current repo/branch and the root hash, mounts
`<dialog-arboretum>`, and answers its lazy expansions by issuing tree-
predicate queries. The view owns tonk wiring; the component owns
rendering, keeping `dialog-arboretum` liftable into dialog-db or elsewhere.

## Phasing

1. **Resolver** — tonk-worker recognizes a string `predicate`, routes to a
   tree resolver that walks via the diagnose decode and emits
   `Conclusion`s for `tree/node` / `tree/child` / `tree/entry` /
   `tree/key`. Verify by querying a seeded branch. Tests like
   `transfer`/`evaluate` (wasm).
2. **`<tonk-key>`** — the key-component bar against sample data.
3. **Outline** — `wa-tree` + range/size bars + lazy expand via
   `tree/child`; leaf → entries. Wrap as `dialog-arboretum`.
4. **Embed** — the `tonk-display` view.
5. **(Optional)** — D3 treemap size overlay.

## Open questions

- **Value decoding at leaves** — how much of the value to decode/show
  inline vs. size-only by default (large values are the perf point).
- **Human-readable resolution** — entity/attribute may need a lookup the
  resolver does not cheaply have; decide what resolves server-side vs.
  what stays a short hash in `<tonk-key>`.
- **Live updates** — static snapshot per revision to start; re-fetch on
  commit (subscription) is later.
- **Future: native dialog predicates** — a storage-backed `Proposition`
  kind in dialog-query would let these chain in one query with real facts
  and load lazily in-engine. Out of scope here; a direction for @cdata.
