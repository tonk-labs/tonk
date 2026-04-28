# tonk-schema — design notes

Typed schema for facts on a repo's `meta` branch, plus the
interpreter that turns asserted-notation `Syntax` into EAV
claims.

## Two halves

### Schema (typed concepts/attributes)

- **`domain.rs`** — `Name`, `Subject`, `Profile`, `Origin`,
  `Upstream`, `Address`. Cross-cutting attributes shared
  across the replica/branch/remote concepts.
- **`replica.rs` / `branch.rs` / `remote.rs` / `tracking_branch.rs`** —
  the meta-branch concepts. Identity is `did:key:z6Mk` +
  base58(blake3(dag_cbor(inputs))) via `EntityExt::of`.
- **`meta.rs`** — `dialog.meta/{name,description}` attributes
  (`Name`, `Description`), the `attribute::{Id, Type, Cardinality}`
  newtypes for the `dialog.attribute/*` namespace, and the
  `Named` / `AttributeFacts` concepts used by the resolver.
- **`concept.rs`** — re-exports `dialog_query::{AttributeDescriptor,
  ConceptDescriptor}` and exposes `with(name)` / `maybe(name)`
  helpers for constructing `dialog.concept.with/{field}` /
  `dialog.concept.maybe/{field}` relations.
- **`rule.rs`** — re-exports `dialog_query::DeductiveRuleDescriptor`.

### Interpreter (`interpret.rs`)

Turns a `tonk_notation::Syntax` into a `Transaction { claims, bookmarks }`.

- **Identity comes from dialog**, not from us. Attributes use
  `AttributeDescriptor::to_uri()` (`the:base58(blake3)`).
  Concepts use `ConceptDescriptor::this()` (`concept:base58(blake3)`).
  Both are the canonical content-addressed URIs dialog already
  defines; the interpreter never invents new schemes.
- **Domain-context entities** use `derive_entity(name)` (blake3
  of the bookmark name, encoded as `did:key:z…` with the
  `0xed01` ed25519 multicodec prefix). Same name → same entity →
  in-place updates via dialog's cardinality-one upsert.
- **Bookmark name claims** (`dialog.meta/name`) attach to whatever
  entity the subject resolved to. The bookmark *follows* the
  canonical entity for content-addressed concepts/attributes;
  it *is* the entity for raw domain writes.

## Resolver

Concept fields in `with` / `maybe` can reference an attribute four
ways:

1. **Inline descriptor** — full `attribute:` body in the value
   slot. Asserted recursively, descriptor in hand.
2. **Local bookmark** — string referencing another `attribute:`
   statement in the same document. Built up incrementally.
3. **Remote bookmark** — string referencing an attribute defined
   in a prior transaction. Resolver does a `Named` query on the
   branch, then `AttributeFacts` to reconstruct the descriptor.
4. **URI literal** — `the:…` URI of a known attribute entity.
   Resolver does an `AttributeFacts` query directly.

The `Resolver` trait has two methods (`resolve_attribute(name)`,
`resolve_attribute_by_entity(entity)`) so all four forms produce
a `ResolvedAttribute { entity, descriptor }`. Without the
descriptor the concept can't hash correctly.

The trait uses dialog's standard async pattern:

```rust
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Resolver { ... }
```

`Send` on native (axum needs it for handler bounds), `?Send` on
wasm (where the worker actually runs).

`NoopResolver` is the no-I/O fallback for tests and document-only
paths.

## Why the descriptor must round-trip the resolver

Dialog hashes a concept over the *attribute URIs* of its required
fields, but the worker stores attributes as
`(dialog.attribute/id, dialog.attribute/type,
dialog.attribute/cardinality, dialog.meta/description)` — four
separate facts. To reconstruct an `AttributeDescriptor` from the
branch we need all four (description is required so the schema
invariant "every named attribute has a description claim" holds
across writes). Hence `AttributeFacts` is a typed `Concept` over
exactly that quadruple, and the resolver assembles them into a
descriptor via `serde_json` round-trip.

## Carry compatibility

- **`dialog.meta/*` and `dialog.concept.with/*` namespaces** match
  carry's. Field-name facts written by either tool describe the
  same concept identically.
- **Bookmark entity URIs** diverge slightly. Carry derives an
  ed25519 *signing* key from the blake3 hash, takes its
  *verifying* key, and encodes that. We encode the hash bytes
  directly as the multicodec payload (no ed25519 derivation), so
  the parser can stay sync. Same URI shape; different bytes for
  the same name.
- **Attribute and concept URIs** (`the:…` / `concept:…`) are
  byte-identical to dialog's canonical scheme, since both crates
  use `AttributeDescriptor::to_uri()` / `ConceptDescriptor::this()`.

The carry migration story when we get to it: rewrite carry's
parser to use `tonk_notation::parse` and `tonk_schema::interpret`,
inheriting the URI shape automatically.

## Anonymous and variable subjects

Both rejected by the interpreter. `_:` because the design discussion
("does it dedupe per-document, per-content, or per-anything?")
isn't settled; `?var:` because that's a query/rule construct we
don't have yet. Parser still tags them in the AST so the
interpreter can produce a clear error.

## Tests

All `interpret.rs` tests use `#[dialog_common::test]`, which
expands to `#[tokio::test]` on native and
`#[wasm_bindgen_test]` on wasm. Same harness as the rest of the
codebase. Coverage walks each reference form (inline / local /
remote / URI) plus failure cases.
