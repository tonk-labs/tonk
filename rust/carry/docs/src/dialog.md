# Dialog DB

Dialog is the embeddable database engine that powers Carry. Understanding Dialog helps explain why Carry works the way it does and what makes it different from other tools.

## What is Dialog?

Dialog is a local-first, semantic database designed for software that works offline, syncs across devices, and keeps data under user control. It is developed as a separate project ([dialog-db](https://github.com/dialog-db/dialog-db)) and Carry is one application built on top of it.

Dialog has three layers:

### Associative Layer (Memory)

The storage layer. All data is stored as **claims** indexed in three key orderings within a single prolly tree:

- **EAV** (Entity -> Attribute -> Value): "What are all the claims about this entity?"
- **AVE** (Attribute -> Value -> Entity): "Which entities have this attribute with this value?"
- **VAE** (Value -> Attribute -> Entity): "What references this entity?"

Retractions remove claims from these indexes. A `Cause` chain links each claim to its predecessor, providing single-link provenance.

### Semantic Layer (Interpretation)

The interpretation layer. Operates at query time, reading schema primitives (attributes, concepts, rules) from the associative layer and using them to interpret data. This is where schema-on-read happens:

- **Attributes** add type and cardinality constraints to relations.
- **Concepts** compose attributes into named schemas.
- **Rules** derive new concept instances from existing data.

When you `carry query person`, the semantic layer reads the `person` concept definition from storage, matches it against existing claims, and assembles the results.

### Reactive Layer (Behavior)

The behavioral layer. Where the semantic layer *interprets* data, the reactive layer *responds* to it. Processes would observe concepts and relations, and in response produce new claims. This is where effects, triggers, and inductive rules would live.

> [!NOTE]
> The reactive layer is not yet implemented. Foundational data structures exist (Z-sets for database stream processing, an `Operator` trait) but there is no wiring to the query engine and no mechanism to trigger rule re-evaluation on data changes.

## Key Properties

### Schema-on-Read

Dialog doesn't require you to define a schema before writing data. You can assert any claim at any time. Schemas (attributes, concepts) are themselves claims in the database -- they're interpreted at query time, not enforced at write time.

This means:
- You can evolve your schema without migrations.
- New kinds of data can be added without redesigning existing structures.
- Queries can interpret the same data through different schemas.

### Retraction Semantics

When you retract a claim, it is removed from the indexes. Each claim records a `Cause` link to its predecessor, providing basic provenance tracking. A full temporal index with time-travel queries is planned but not yet implemented.

### Content-Addressed Identity

Attributes and concepts derive their identity from the hash of their content. Two attributes with the same relation, type, and cardinality will have the same DID, regardless of when or where they were created. This gives you convergent identity without coordination.

Note: Dialog DB itself creates regular entities with random keypairs. Carry adds content-addressed entity identity on top -- when you `carry assert`, the entity DID is derived from a BLAKE3 hash of the asserted fields. This is a Carry-level behavior, not a Dialog DB primitive.

### Structural Sync

Dialog uses prolly trees for synchronization. When two replicas diverge and later sync, efficient structural diffs identify the changes and merge them automatically. Conflicts are resolved deterministically via hash-based tiebreaking, ensuring all replicas converge to the same state.

## Anatomy of a Claim

A claim consists of:

```
the: <relation>      -- domain/name identifying the kind of association
of:  <entity>        -- the entity this claim is about (a DID)
is:  <value>         -- the value being associated
```

In Carry's YAML output:

```yaml
- the: com.app.person/name
  of:  did:key:zAlice
  is:  Alice
```

## Primitive Domains

Dialog reserves several domains for internal use:

| Domain | Purpose |
|---|---|
| `dialog.attribute` | Attribute identity: `/id`, `/type`, `/cardinality` |
| `dialog.concept.with` | Required concept fields |
| `dialog.concept.maybe` | Optional concept fields |
| `dialog.meta` | Universal metadata: `/name`, `/description` |
| `dialog.rule` | Rule definitions: `/deduce`, `/when`, `/unless`, `/where`, `/assert` |

All domains starting with `dialog.` are reserved. User-defined domains must not use this prefix.

## Further Reading

- [Anatomy of Dialog](https://gozala.io/dialog/anatomy) -- the authoritative design document
- [Dialog DB on GitHub](https://github.com/dialog-db/dialog-db) -- the source code and architecture decision records
- [Datalog](https://en.wikipedia.org/wiki/Datalog) -- the query language family that inspired Dialog's rule system
