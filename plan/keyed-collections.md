# Keyed collections in the concept DSL

Status: **landed end to end.** Dialog PR #472
(`feat/keyed-collection-types`) queries and induces collection fields;
the tonk side declares them (`as: {[position]: entity}`), binds entries
as `{?key: ?value}` in queries, premises, and assertions, and folds a
subscription's rows into one `{key: value}` map per field. The notebook
orders its blocks through `block: {[position]: entity}`: the element
derives a position between a block's neighbours with
`dialog_artifacts::position` and dispatches one `block/place` per block
that moved, so a reorder touches only what moved. Inside a `{block}`
iteration a template reads the entry's key as `{block/key}`.

Two things settled during implementation, beyond the design below:

- **The key rides as an entry, not an operand an author names.** In
  notation and on the wire a collection field is bound as
  `{the: <key>, is: <value>}`; `{?key: ?value}` is sugar for it. The
  engine keeps two operands (`block`, `block/key`) but only ever
  shows them in error output.
- **Induction is first-class.** A rule's reach is an attribute or a
  keyed half of a domain; the trigger index files a collection
  premise under the half's cover key, and an inductive head over a
  collection writes `domain/key`. Nothing about collections is
  refused at rule compile any more.

## What this is

A concept field that holds *many* keyed entries rather than one value:

```yaml
concept!: &notebook
  with:
    title:
      the: xyz.tonk.notebook/title
      as: text
    block:
      description: The notebook's blocks, in document order.
      the: xyz.tonk.notebook
      as: {[position]: entity}
```

`block` is not one fact. It is every fact in the `xyz.tonk.notebook`
domain whose attribute name half is a position, keyed by that position
and valued by an entity.

Note what `the:` is here: a **domain**, not a full attribute. A scalar
field names `xyz.tonk.notebook/title`; a collection field names
`xyz.tonk.notebook` and the key supplies the name half. That is the
whole trick, and it is already how the substrate stores ordered
relations.

## Why the key type is written in the value position

Two shapes, distinguished by the key type:

```yaml
as: {[position]: entity}   # a sequence — ordered, position-keyed
as: {[symbol]: entity}     # a dictionary — named, symbol-keyed
```

`position` and `symbol` are **types**. They dictate the shape only
because `dialog_artifacts::Name` has exactly those two variants, and
they are disjoint by first byte (positions open with an uppercase
major, symbols with lowercase). One domain scan therefore yields both
kinds of entry already sorted, and classification needs no tag — see
`Directory::admit` / `Sequence::admit`.

The declaration is braced because the *query* form is braced. That
consistency is the argument for it:

```yaml
as: {[position]: entity}   # declaration
display: { ?name: ?view }  # query
```

The cost, acknowledged: at a glance `block` reads like a scalar field,
because the collection-ness lives in `as:` rather than in the field
name. `description:` carries the human signal. The alternatives
considered were `block[position]:` (marks the field name, most visible,
but breaks the declaration/query symmetry) and a separate `each:` block
beside `with:` and `maybe:` (most visible, but splits the concept's
fields across two blocks).

## Cardinality is orthogonal

`cardinality:` keeps its current meaning and default of `one`. It is
**per entry**, not about the collection: whether one entity may have
two `tonk.view/basic` facts or just one. A collection field is
many-*entried* by construction; each entry is independently
cardinality-one or cardinality-many. Writing `cardinality: many`
alongside `as: {[symbol]: entity}` is meaningful, not contradictory.

## Querying: the key is an ordinary pattern position

A literal key matches one entry; a variable key matches every entry.

```yaml
# one named view
view:
  this: ?model
  display:
    basic: ?view

# every view, with its name bound
view:
  this: ?model
  display:
    ?name: ?view
```

This is not new machinery. It is the rule from `@gozala/2026-05-16.md`:

> a literal key can only match one entry, a variable key can only mean
> "every entry." The pattern itself carries the multiplicity.

That entry wrote it about macro patterns and listed the query side as
open item #1 ("the assertion side is well-defined now; the query side
is not"). The same rule turns out to serve both, which is what closes
it.

`?name` binds as an ordinary logic variable — it joins, filters, and
appears in conclusions like any other. No accessor syntax, no `@`, no
`.at`, no virtual entity for the entry. Key and value are bound in one
pattern, flat. This is Soufflé's destructuring-at-the-binding-site in
YAML's shape.

## What this replaces

**Named views.** Today a model gets one `view!` and one
`view/directory!`, the latter a separate keyword encoding "the
entity-unset variant". Two directory views on one model conflict,
because there is no way to name them apart. As a dictionary field they
are just entries:

```yaml
view!:
  this: model
  display:
    basic: <h1>Hello</h1>
    list:  <ul>…
```

`view/directory!` becomes an ordinary `directory:` entry beside the
others, and `<tonk-display view=list>` picks one. One less special
case, and "query for all views of this model" is `?name: ?view`.

**The notebook's order key.** `xyz.tonk.notebook.block/order` as a
`text` field on each block, with the element sorting rows by
`data-order`, is a hand-rolled sequence. As a position-keyed field the
ordering is the scan order and `dialog/position` derives insertion keys.

## Machinery that already exists

- `dialog_artifacts::position` — fractional positions, `Bias::derive`,
  `insert(bias, range)`.
- `dialog/position` formula — derives a position for a member inserted
  between two neighbours. Deterministic, so concurrent identical
  inserts converge.
- `dialog/position-parts` formula — splits an attribute into
  `namespace` + `position`, and doubles as the filter selecting a
  scan's ordered members (ordinary word predicates project nothing).
- `Directory<T>` / `Sequence<T>` in `dialog-artifacts/src/collection.rs`
  — `BTreeMap<Symbol, T>` and `BTreeMap<Position, T>`, with `admit`
  classifying a scanned claim by name shape.

## What the query layer already has

This section replaces an earlier reading of mine that said the gap was
on the dialog side. It is not. Checked against `origin/main`
(`59b1a2f1`), which is two commits past our pin and touches nothing
here — the machinery below is all present in `tonk-2026-08-25b`.

`DynamicAttributeQuery` takes `the` as a **`Term<The>`**, so the
attribute can be a *variable*. When it is, the lowering to a selector
(`attribute/query/all.rs:387-417`) reads two refinements off the
variable's kind:

- **`prefix`** — becomes an AEV range bound
  (`ArtifactSelector::the_starting_with`). A whole domain like
  `"todo.list/"` is exactly the collection field's `the:`.
- **`name_shape`** — `NameShape::Position` or `NameShape::Symbol`,
  becomes `ArtifactSelector::with_name_shape`, narrowing the scan to
  that shape's contiguous half of the domain range.

Composing them is the whole feature, and it is a supported path with
tests at every level:

```rust
let members_kind = Kind::from(Type::Symbol)
    .with_prefix("todo.list/")?
    .with_name_shape(NameShape::Position)?;
AttributeQueryAll::new(
    Term::<The>::var("a").with_kind(members_kind),
    Term::<Entity>::var("e"),
    Term::var("v"),
    Term::var("cause"),
)
```

`artifacts.rs:1516` proves it end-to-end against a real store: one
domain holding `todo.list/title`, `todo.list/owner`, `todo.list/N`,
`todo.list/N5` yields exactly `["todo.list/N", "todo.list/N5"]` under
a Position shape and exactly `["todo.list/owner", "todo.list/title"]`
under a Symbol shape — the ordered half in list order, the dictionary
half in name order.

So `{[position]: entity}` and `{[symbol]: entity}` are not a new engine
capability. They are **notation for a query the engine already
plans and executes**: bind `the` to a variable, refine it with the
field's domain as prefix and the key type as name shape, and bind the
key variable to the same term. `?name` in `{?name: ?view}` is that
attribute variable; `dialog/position-parts` splits it into namespace
and position when the position itself is wanted as a value.

`Directory<T>` / `Sequence<T>` in `dialog-artifacts` are a separate,
optional convenience — an aggregation target for collecting a scan
into a map. They have one caller in the whole repo (a test at
`dialog-repository/.../session.rs:2156`) and **are not on the critical
path**: a conclusion binding `(?name, ?view)` pairs is already the
useful shape for rendering.

`notes/record-value.md` does not help here and should not be confused
with it: a `Value::Record` is deliberately opaque to the query layer
("carries it, stores it, compares it by bytes, but never looks
inside"). Records solve *atomicity* — one fact, many fields, written as
a unit. Collections solve *multiplicity*. Different axes. `RecordFormat`
is also unimplemented; the note is a decision doc.

## The descriptor now holds a collection

`AttributeDescriptor.the` was a `The`, which validates as exactly one
`/` — so a collection field's domain-only relation could not be stored
at all. It is now an enum:

```rust
pub enum Relation {
    Attribute(The),
    Collection { domain: Symbol, keyed: Keyed },
}
pub enum Keyed { Dictionary, Sequence }
```

The key kind is the variant rather than a field beside one, so a
collection cannot be described with a key kind that disagrees with it —
there is no state left to validate.

**`Relation::term()` is the seam.** An attribute lowers to
`Term::Constant`; a collection lowers to a variable refined by the
domain prefix and name shape — the query the wire format spells and
that the round-trip test proved runs. Three call sites route through
it: `rule/deductive.rs` (concept field to premise), `query/typed.rs`,
and the `#[derive(Concept)]` macro. So a collection field becomes a
real ordered scan wherever a concept becomes a query.

**Write paths refuse rather than fabricate.** `Relation::attribute()`
returns `Option`, and `resolve()` errors with `UnkeyedCollection`: a
collection describes many facts, and the key belongs to the entry, not
the schema.

**Identity is preserved.** `to_cbor_bytes` hashes
`{domain, name, cardinality, type}`, and a plain attribute encodes
byte-identically to before — every existing attribute identity is
unchanged. A collection puts `<dictionary>`/`<sequence>` in the `name`
slot, which is the same slot because it answers the same question:
what the name half of these facts holds.

**Stored documents still load.** `#[serde(untagged)]` means
`"the": "person/name"` parses straight into `Relation::Attribute`.

## A rule cannot derive into a collection

Induction tracks a rule's reach as a `BTreeSet<Attribute>` (the
`touched` sets in `transaction::induce`), and a collection spans a
domain rather than naming one attribute. A rule concluding into one
would write facts nothing recorded it had touched, so its consumers
would never re-evaluate — silently stale derived data.

`analyze_with`, the gate every rule passes through, now rejects it with
`AnalysisError::CollectionHead`. The induction sites assert instead of
skipping, pointing at that refusal for why they cannot be reached.
Reading a collection in a premise stays fine — `premise_attrs` skips
it, because a collection contributes no single attribute to a set of
attributes.

Widening those sets to hold domains alongside attributes is the real
answer, and would let a rule derive into a collection. Nothing needs it
yet; refusing loudly beats skipping quietly meanwhile.

## The query endpoint already carries it

`Query.terms` is a `Parameters` map of `Term<Any>`, and `Term`'s serde
writes the variable's **full `type_system::Type`**, refinements
included — unlike `AttributeDescriptor`, whose `as:` is the flat
`ValueDataType`. So the two halves of a query diverge:

- **`Query.terms`** — carries prefix + name shape today. No dialog
  change needed.
- **`Query.predicate`** (a `ConceptDescriptor`) — still cannot express
  a collection field, per the two gaps above.

Verified end to end against a real branch (test landed in the meg
worktree as `it_preserves_a_name_shape_scan_across_a_json_round_trip`,
`dialog-query/src/attribute/query/all.rs`): a refined term serialized
to JSON, parsed back, still drives the scan.

```
position -> ["Bread", "Milk"]      both ordered members
symbol   -> ["Groceries"]          the dictionary entry
```

So a keyed-collection query is expressible over the wire **now**. What
is blocked is *storing* a concept whose field is a collection.

## The wire format (revised, dialog-side)

The form that worked was not one to build on:

```json
{"?":{"name":"a","type":{"refined":[{"bits":128},
  {"prefix":"todo.list/","name_shape":"position"}]}}}
```

`{"bits":128}` publishes an enum discriminant (`1 << 7` = Symbol), so
renumbering silently changes every stored document. `name_shape` is
snake_case alone in a kebab format. And `[base, refinement]` is
positional, carrying no hint of what either half is. This is not
special to collections — EVERY typed variable serialized this way.

The replacement, on branch `feat/keyed-collection-types` off dialog
`origin/main`:

```json
{"?": {"name": "a",
       "where": {"type": {"symbol": {}},
                 "domain": {"is": "xyz.tonk.notebook"},
                 "name": {"case": "position"}}}}
```

**Two container conventions**, chosen to match how the values combine
rather than as arbitrary style:

- **object = union**, entries are alternatives of one kind. `type` is
  the case: intersecting two sets keeps the shared keys, which is
  exactly `Primitive::intersect` on the bits.
- **array = intersection**, entries are independent obligations. `as`
  (conformance) is the case: every listed concept must hold.
- **a record of fixed slots is neither.** `where` is one: each slot
  appears at most once and merges rather than accumulates.

**The ten type variants**: `bytes`, `entity`, `boolean`, `text`,
`uint`, `int`, `float`, `record`, `symbol`, `option`. Each valued by a
parameter record, empty today — the place a future integer width would
go. `option` is the synthetic absence atom (bit 9): present alongside
others it marks the variable optional.

**The constraint slots**: `domain` `{is}` (un-slashed — the separator
is an encoding detail supplied on the way in), `name` `{case}`
(`position` | `symbol`), `starts-with`, `as` (array), and the bounds
`>=` `>` `<=` `<`.

**Loose in, normalized out.** The derived `{"primitive": …}` /
`{"refined": …}` forms still parse, so a rule stored before this
loads unchanged; only the named form is ever written. That removes the
migration this would otherwise need.

Two design points worth keeping:

*Refinements attach to the SET, not to a member.* An earlier draft
nested constraints under each variant — `{"symbol": {"domain": …}}` —
which is more honest about which constraints apply to which types. It
breaks on `Type::optional()`, which produces `Refined(Text|Nothing,
{prefix})`: one refinement over a two-member union. That is not an edge
case, it is what every optional refined field is, so the flat form wins.

*A whole-domain prefix is written as a `domain`.* `apply_prefix_bounds`
narrows the scan to the name shape's byte class only when the prefix
ends at `/`; a prefix one byte short silently degrades to a per-row
filter. Writing the domain without its separator makes that
unrepresentable.

## Subscriptions: verified, was untested

`Demand::record` -> `selector_range` -> `apply_prefix_bounds`, which
narrows to the shape's first-byte class. So a subscription over a
sequence demands only the positions half of its domain.

This was correct by construction and had **zero test coverage** —
`grep name_shape` across `dialog-repository` returned nothing. Three
tests added:

- a symbol-named write in the same domain does not wake an ordered
  subscription (the halves are genuinely separate covers)
- a position-named write does (the complement, so the first test
  cannot pass by the subscription being broken)
- a domain scan does not trip the head gate (which would silently turn
  incremental maintenance back into polling)

## Tonk-side seams, in dependency order

1. **`tonk-notation` parse.** YAML itself is fine with both forms —
   verified against saphyr, our parser. `{[position]: entity}` parses
   cleanly (YAML permits complex keys, and a bracketed key is just a
   sequence-valued key), and `{?name: ?view}` parses today with **zero
   diagnostics**. The only obstacle is our own check: `walk_field`
   (`parse.rs:825`) requires a string key and rejects everything else
   with "Field name must be a string." So the query side needs nothing
   from the parser, and the declaration side needs a bracketed-key arm
   in `walk_field` plus a variant in `FieldValue` to carry it.

2. **`tonk-analyzer` declaration.** `normalize_type_name`
   (`declaration.rs:879`) maps seven scalar type names and returns
   `None` for everything else. `stringify_simple_value` explicitly
   rejects `FieldValue::Nested` for `as:` slots. Both are the seam a
   collection type has to pass.

3. **`tonk-schema` descriptor.** `build_attribute_descriptor`
   (`concept.rs:467`) writes `as:` as a plain string into the
   `AttributeDescriptor` shape. Needs a form carrying the key type
   alongside the value type, and `the:` as a domain — which is the
   dialog-side change above. This is the seam that blocks, and it is
   the one to prototype first.

4. **Formula registry drift** (pre-existing, worth fixing regardless).
   `tonk-analyzer/src/analyzer/formula.rs:103` lists 17 formulas;
   dialog's `define_formulas!` lists 21. Missing: `dialog/revision`,
   `dialog/revision-parent`, `dialog/key-part`,
   `dialog/separator-part`, `dialog/position`,
   `dialog/position-parts`. The comment above `build_registry` says
   both tables "must list the same formulas under the same names";
   nothing enforces it.

## Order of work

Seam 4 is independent, small, and useful on its own — the analyzer
cannot currently see the position formulas at all, so nothing that uses
them can be written with diagnostics. Do it first.

Seams 1–3 then hang together. Because the engine already plans this
query, the honest order is to prove it end to end **from tonk** before
changing dialog at all:

1. Hand-build the refined `AttributeQueryAll` from tonk (prefix +
   name shape) against a real branch, and confirm a notebook's blocks
   come back in position order. No notation, no descriptor — just the
   query. This validates the whole premise cheaply.
2. Only once that renders, decide the descriptor shape, and take the
   `The`-admits-a-domain change to dialog with a working consumer
   already in hand.

That order matters: the dialog change is small but it is a storage
format touching descriptors, and it should be justified by something
that demonstrably works rather than by a design sketch.
