# Glossary

Everything on a branch is a **claim** (a **fact**): an entity, a relation, and
a value. A **relation** is a named set of claims, such as `person/name`; each
claim is one member. An **attribute** is a relation qualified by a value
**type** and a **pick**: `person/name` read as `text` under `last`. A
**concept** is a named schema over attributes. An entity matching a concept is
an **instance**, and the concept presents its attributes as typed **fields**:
a field is a concept's slot holding an attribute. A **view** is an HTML
template rendered over a concept's instances.

A **type** is named by a built-in anchor: `text`, `integer`, `natural`,
`float`, `boolean`, `bytes`, `entity`, `symbol`, `record`
(`signed-integer` and `unsigned-integer` are aliases of `integer` and
`natural`). Each names the entity dialog knows the type by, `text:` for
`text`. A document that declares an anchor of the same name gets a warning,
and within that document the name means its declaration.

A **pick** says which of an entity's claims in the relation a read returns:
`last` (the default) the newest, `all` every one, `top` the best ranked of the
values listed in `as:` (a list implies `top`), `max` and `min` the greatest
and least. Like a type, each pick is a built-in anchor naming the entity
dialog knows it by, `all:` for `all`, and a document anchor of the same name
shadows it.

An **assertion** adds a claim. Creating a new content-addressed instance is
also called **minting**. Under any pick but `all`, a later assertion
**supersedes** the claim a read returns; under `all` claims accumulate. A
**retraction** is itself a claim that invalidates an earlier claim, not an
in-place deletion.

Notation queries are pattern matching with unification. A **rule** has a
premise and a head; transient command facts produced by DOM events can trigger
rules that assert or retract durable facts.

Two consequences matter early:

- Entity identity is content-addressed. Reasserting an identical body is a
  no-op; changing any field creates a new entity unless the old one is bound
  with `this:`.
- Bare lowercase tokens are symbols resolved through the name table or the
  built-in anchors (types and picks), and one that names nothing is an error.
  Quote every string literal: `name: "alice"`, not `name: alice`. The one
  exception is `the:`, which spells a relation by its own name
  (`the: person/name`); a relation is its name, not an entity a name refers
  to.
