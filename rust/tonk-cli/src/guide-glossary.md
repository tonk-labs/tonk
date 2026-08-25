# Glossary

Everything on a branch is a **fact**: an entity, an attribute, and a value.
A **concept** is a named schema over attributes. An entity matching a concept
is an **instance**, and the concept presents its attributes as typed **fields**.
A **view** is an HTML template rendered over a concept's instances.

An **assertion** adds a claim. Creating a new content-addressed instance is
also called **minting**. On a cardinality-one field, a later assertion
**supersedes** the old value; cardinality-many fields accumulate values. A
**retraction** is itself a claim that invalidates an earlier claim, not an
in-place deletion.

Notation queries are pattern matching with unification. A **rule** has a
premise and a head; transient command facts produced by DOM events can trigger
rules that assert or retract durable facts.

Two consequences matter early:

- Entity identity is content-addressed. Reasserting an identical body is a
  no-op; changing any field creates a new entity unless the old one is bound
  with `this:`.
- Bare lowercase tokens are symbols resolved through the name table. Quote
  every string literal: `name: "alice"`, not `name: alice`.
