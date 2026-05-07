# Asserted-notation guide

A YAML-flavoured DSL for reading and writing facts about
entities in a tonk repository. One document is a sequence of
expressions; each expression is a query, an assertion, or a
retraction. The worker's `/evaluate` route runs a whole
document in one transaction, returning matches and a commit
summary.

This guide builds up examples that you can paste into the
editor in order — each one runs against the branch the previous
ones left behind.

## Two flavours of expression

Every top-level entry in a document is one of two things,
distinguished by the `!` suffix on the head:

| Head form | Body            | Meaning       |
|-----------|-----------------|---------------|
| `head:`   | fields or empty | **Query**     |
| `head!:`  | fields or empty | **Assertion** |

Retraction is not a separate top-level shape — it happens
*inside* an assertion body, by giving a field the blank value
`_` (retract that one attribute) or by adding `..: _` (retract
every attribute in the concept's `with:` map that isn't named
explicitly elsewhere in the body). See *Blanks* below.

Heads carry only the concept (or claim domain) and the effect
marker. They have no bindings; everything about *which* entity
an expression operates on lives in the body.

## A model of names

Names in this notation are themselves entities, in the same
sense as Git branches and tags. A name is an entity whose only
job is to point to another entity. Each name has at most one
target at a time (cardinality one), but a target may be reached
through any number of names.

Two ways to refer to a name appear throughout the notation:

- **Bare symbol** (`person`) — resolves whatever entity the name
  `person` currently points to. This is what you write when you
  want the target entity, not the name. Most use sites are bare
  symbols.
- **URI** (`id:person`) — direct reference to the name entity
  itself, with no resolution. Use this when you mean the name as
  an object — to rename it, alias it, query it, or operate on it
  as the head of an assertion when bare-name resolution would be
  wrong.

The `id:` scheme is where user-published names live. Anchors on
the head's value (`&person`) publish to it.

A third shape, the anchor `&person`, is *sugar* for publishing —
it's not a way to refer to a name, it's a way to create one. The
desugaring is in *Names and references* below.

URIs come in several schemes:

- `id:foo` — user-published name entities.
- `db:foo` — system-published built-in entities (`db:concept`,
  `db:attribute`, `db:name`). The `db:` scheme is reserved and protected
  against user assertions; nothing the user does can change what
  lives at these URIs.
- `did:key:…` — content-addressed entities (DIDs).
- `xyz.tonk.person/name`, `dialog.meta/name`, etc. — attribute
  URIs in `domain/name` form.

All of these are direct references and require no resolution.

## Worked tutorial

The shortest path from "empty branch" to "you can read and
write your own data." Run the snippets in order.

### 1. Define an attribute

`attribute!` is the built-in concept for declaring an attribute.
The body says where the attribute lives (`the:`), what kind of
value it carries (`as:`), and how many values per entity
(`cardinality:`). The `&person-name` anchor publishes the
resulting attribute entity under the name `person-name`, so
future revisions can reference it by that bare symbol.

```yaml
attribute!: &person-name
  description: "The person's name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
```

After the commit, the URI `id:person-name` is an entity pointing
to the new attribute entity. The bare symbol `person-name`,
when resolves to the attribute entity itself.

### 2. Define more attributes

Each `attribute!` concept produces one attribute entity. You can
have many in one document; they all commit in one transaction.

```yaml
attribute!: &person-age
  description: "The person's age in years"
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one
```

### 3. Define a concept

`concept!` composes attributes into a named shape. Its `with:`
block maps field names to attributes. References are written
as bare symbols, which resolve through the name table.

```yaml
attribute!: &person-name
  description: "The person's name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one

attribute!: &person-age
  description: "The person's age"
  the:         xyz.tonk.person/age
  as:          unsigned-integer
  cardinality: one

concept!: &person
  description: "A person"
  with:
    name: person-name
    age:  person-age
```

The bare symbols `person-name` and `person-age` in the `with:`
block resolve against the names published earlier or in the same
document.

### 4. Add a person

`person!:` is asserts the `person` concept. Use `&alice` to name
the resulting entity `alice`:

```yaml
person!: &alice
  name: "Alice"
  age:  28
```

The semantics:

- The entity is derived from the asserted payload
  (`Entity::of(&{name: "Alice", age: 28})`).
- Derived entity gets associated with `id:alice`.
- Re-running the same body is a no-op.
- Re-running with a *different* body produces a different
  entity and associates `id:alice` to the new one. The previous
  entity still exists with its claims; but the name
  no longer resolves to it.

### 5. Read it back

Anonymous query — `person:` matches every entity satisfying
the `person` concept's schema and surfaces every field:

```yaml
person:
```

Constrain the match by giving fields literal values:

```yaml
person:
  name: "Alice"
```

Bind the matched entity to a variable using `this:`, and captures it under a
variable so it could be used in other expressions.

```yaml
person:
  this: ?p
  name: "Alice"
  age:  ?age
```

`?p` and `?age` come back in the result frame.

### 6. Update Alice

Two distinct operations, syntactically different.

**Re-point the name** to a new entity (same name, different
target). Reuse the anchor with a different body:

```yaml
person!: &alice
  name: "Alice"
  age:  29
```

This produces a *new* entity (different body hash). The name
`id:alice` is re-pointed to it.

**Update the same entity**. Use a query to bind the entity to
a variable, then assert against the variable:

```yaml
person:
  this: ?alice
  name: "Alice"

person!:
  this: ?alice
  age:  30
```

The query binds `?alice` to every entity currently named
"Alice" in the `person` concept. The assertion writes
`age = 30` on each match. Because `xyz.tonk.person/age` is
`cardinality: one`, the prior age claim is auto-retracted as
part of the transaction.

### 7. Retract Alice's whole projection

Retraction always happens inside an assertion body, against an
entity selected via `this:`. The blank value `_` does the work.
Two shapes cover all cases:

- `field: _` — retract just that one attribute.
- `..: _` — retract every attribute in the concept's `with:` map
  that isn't named explicitly elsewhere in the body.

To retract a single field:

```yaml
person!:
  this: ?alice
  age: _
```

To retract every attribute in the concept (full concept-level
retraction), combine `..: _` with a query that binds the entity:

```yaml
person:
  this: ?alice
  name: "Alice"

person!:
  this: ?alice
  ..: _
```

The worker queries the branch for every fact whose attribute is
in the `person` concept's `with:` map and whose subject is the
matched `?alice` entity, and dissociates each match.

To retract every attribute *except* the ones you specify,
combine explicit fields with `..: _`:

```yaml
person!:
  this: ?alice
  name: ?name
  ..: _
```

This preserves `name` and retracts every other attribute in the
concept's `with:` map.

You can name the entity directly without a prior query — by URI:

```yaml
person!:
  this: did:key:zHjKf…
  ..: _
```

Or by name:

```yaml
person!:
  this: alice
  ..: _
```

Retraction by name follows the name's current redirection. If
`id:alice` was re-pointed since the data was written, the
retraction targets the entity it currently points to, not the
original.

## Symbols and strings

The notation distinguishes symbols from strings lexically.

A **symbol** is a sequence of ASCII characters that:

- Starts with a lowercase letter (`a`–`z`).
- Continues with lowercase letters, digits, `-`, `.`, `+`.
- May contain at most one `/` for namespace separation.
  Neither the prefix nor the name part can contain a further
  `/`.

Symbols intended for use as **anchor names** (and thus as the
local part of `id:` URIs) follow a stricter rule: no `/` is
permitted, since the resulting URI must be a valid scheme-form
identifier.

A **string** is any value that:

- Is enclosed in single or double quotes (`'Alice'`,
  `"Alice"`).
- Or contains characters outside the symbol charset
  (uppercase, spaces, punctuation other than `-`, `.`, `+`,
  `/`).

The boundary case worth remembering: a value that *could* be
parsed as a symbol but is meant to be a string MUST be quoted.
`name: alice` is a symbol (resolves through the name table);
`name: "alice"` is a literal string. The quotes are
load-bearing.

## Heads in detail

A head is a name plus the optional effect marker:

```
<name>[!]
```

### Name

- A bare identifier (`person`, `attribute`, `concept`,
  `name`) is a **concept** name. The analyzer resolves it via
  the name table. The built-in concepts `attribute`,
  `concept`, and `name` are mapped by default to
  `db:attribute`, `db:concept`, `db:name`; user-defined
  concepts must have been asserted earlier (or earlier in the
  same document).
- A reverse-dotted identifier (`xyz.tonk`, `io.gozala.person`)
  is a **claim** domain. Each field name combines with the
  domain to form an attribute URI (`xyz.tonk/role`). Claim
  heads have no schema, so the body must enumerate every
  field you care about — `xyz.tonk:` with no body is a parse
  error.
- A scheme-prefixed URI (`db:concept`, `id:person`,
  `did:key:…`) is a direct entity reference, used as the head
  with no name resolution. Useful when bare-name resolution
  would resolve to the wrong thing — for example, if a user
  has shadowed the bare name `concept`, `db:concept!:` still
  reaches the built-in.

### `!` — effect marker

The trailing `!` marks the expression as having an effect.
Without it, the expression is a query (read-only). With it,
the expression contributes to a transaction (assertion or
retraction).

### Anchors on the head's value

A YAML anchor written between the head's colon and its body
publishes the resulting entity under that name:

```yaml
attribute!: &person-name
  ...
```

Anchors are unique within a transaction (one document
submitted to `/evaluate`). To publish the same entity under
multiple names, use explicit `name!` expressions (see *Names
and references* below).

YAML aliases (`*name`) are not used in this notation. The
analyzer rejects any document containing one.

## Bodies

A body is one of:

- **A mapping of fields** — most expressions.
- **Empty** — `head:` or `head!:` with no fields. On a query,
  matches any entity satisfying the concept's schema. On an
  assertion, an empty body has no claims to assert and is a
  no-op (accepted but flagged by the analyzer).

Within a mapping body, two reserved meta-keys do meta work:

- **`this:`** — selects the entity the expression operates
  on.
- **`..: _`** — rest-of-attributes retraction. On `head!:`,
  retracts every attribute in the concept's `with:` map that
  isn't explicitly set elsewhere in the body. The `..` key is
  reserved; it cannot appear with any other value.

If `this:` is omitted from an assertion, the entity is derived
from the content. If omitted from a query, `this` is a free
variable matching any entity.

## The `this:` meta-key

`this:` selects which entity the expression operates on. Its
value can take four forms:

| Value form          | Meaning                                                 |
|---------------------|---------------------------------------------------------|
| Omitted             | Entity is content-addressed from the body               |
| `?var`              | Logic variable — bind/unify across expressions          |
| `name` (bare)       | Resolve through the name table to a target entity       |
| `did:key:…`         | Entity URI directly (no resolution)                     |
| `{ ... }` (mapping) | Entity is content-addressed from the mapping content    |

The mapping form lets you control entity derivation
explicitly:

```yaml
person!:
  name: Alice
  age: 23
  this:
    entropy: Maybe Not
```

By default the entity is derived from the body fields,
so two assertions with identical bodies produce the
same entity. Adding `this:` with a mapping replaces this
default: the entity is derived from the mapping's content
instead, decoupling identity from body content. The
`entropy:` field above is an example — a salt that
ensures distinct entities for otherwise-identical bodies.

## Field values

Fields go on the right of `field:`. Seven flavours,
distinguished lexically:

| Source                          | Meaning                                                    |
|---------------------------------|------------------------------------------------------------|
| `"Alice"` (quoted)              | String literal                                             |
| `28`, `1.5`, `true`             | Primitive literal                                          |
| `?name`                         | Logic variable                                             |
| `_`                             | Blank — query: match any value; assertion: retract field   |
| `person-name` (bare lowercase)  | Symbol — resolves through the name table to a target entity |
| `id:foo`, `db:foo`, `did:key:…` | URI — direct entity reference, no resolution               |
| `xyz.tonk/foo`                  | Attribute URI — direct, no resolution                      |

The distinction between a bare symbol and a URI is
load-bearing. `name: person-name` says "this field's value is
the entity that the name `person-name` currently points to."
`name: id:person-name` says "this field's value is the name
entity itself." Most user data wants the first; the second is
for the rare cases where you need to manipulate the name as
an object (rename it, query its target, etc.).

## Variables

`?name` is a logic variable. Two rules:

1. **In a query**, it binds whatever value matches the
   position.
2. **Across expressions in one document**, occurrences of the
   same variable name unify — the same value must satisfy
   every position the variable appears in.

```yaml
# Find people whose name matches their xyz.tonk role.
person:
  this: ?p
  name: ?n

xyz.tonk:
  this: ?p
  role:   ?n
```

Variables introduced by `&` anchors are document-scoped: an
anchor `&person-name` binds `?person-name` for use in
subsequent expressions in the same document.

Variables in an assertion or retraction body that aren't
bound by some query expression are accepted in `this:`
position (where they introduce a new content-addressed
entity registered under the variable name) but rejected in
field positions.

## Blanks (`_`)

`_` only appears in field-value position. The meaning depends on
the expression flavour:

1. **Query** (`head:`) — match any value. The matched value
   isn't surfaced as a join key; if you want to refer to it
   later, use a named variable (`?x`).
2. **Assertion** (`head!:`) — retract the named attribute for
   the entity selected by `this:`.

The reserved field name `..` accepts only `_` as its value, and
only inside an assertion body. `..: _` retracts every attribute
in the concept's `with:` map that isn't named elsewhere in the
body. A bare `_` at the body level (`head!: _`) is a parse
error — entity selection requires a `this:` field, which
requires a mapping body.

## Joins

A document may contain multiple expressions. Variables
with the same name across expressions join. Results are
filtered to bindings that satisfy every expression
simultaneously.

```yaml
person:
  this: ?e
  name: ?name

xyz.tonk:
  person: ?e
  role:   ?role
```

A query+mutation document also joins via shared variables:

```yaml
person:
  this: ?alice
  name: "Alice"

person!:
  this: ?alice
  age:  30
```

For each match of `?alice` from the query, the assertion
fires once with that entity bound.

Joins are limited to a single evaluation scope.

## Names and references

The `&` anchor on a head's value is sugar for an explicit
`name!` operation. The desugaring:

```yaml
attribute!: &person-name
  description: "The person's name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one
```

is equivalent to:

```yaml
attribute!:
  this: ?person-name
  description: "The person's name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one

name!:
  this:   id:person-name
  entity: ?person-name
```

The anchor binds `?person-name` to the body-derived entity,
and a `name!` expression establishes the URI `id:person-name`
as a name pointing to it.

Use `name!` directly when you need to publish under multiple
names, rename, or alias:

```yaml
attribute!: &person-name
  description: "The person's name"
  the:         xyz.tonk.person/name
  as:          text
  cardinality: one

# Additional names for the same entity:
name!:
  this:   id:tonk-person-name
  entity: ?person-name

name!:
  this:   id:p-name
  entity: ?person-name
```

### Cardinality and what `this:` is for

A name has cardinality one on its target — exactly one entity
at a time. Two consequences:

- Anchors only attach to expressions that produce a single
  entity. Assertions do (one body-derived entity per
  expression). Queries don't (multiple matches), so anchors
  on queries are a parse error.
- `name!` typically takes `this:` to specify which name URI
  is being asserted. Writing a `name!` body without `this:`
  is possible but pointless: the body's `entity:` claim would
  attach to a content-addressed `did:key:…` entity, which
  defeats the purpose of having a stable, human-readable
  name. The analyzer accepts the form for grammatical
  consistency but flags it as suspicious.

### Renaming

Renaming is `name!` plus retraction:

```yaml
# Re-point id:alice to a different entity:
name!:
  this:   id:alice
  entity: did:key:zNewAliceEntity

# Or remove the name entirely:
name!:
  this: id:alice
  ..: _
```

The cardinality-one constraint ensures the old binding is auto-retracted
when the new one is asserted.

## Built-in concepts

Three built-in concepts are: `attribute`, `concept`,
and `name`. They define the schemas for attributes, concepts,
and name references.

The built-in entities live at fixed URIs in a reserved
URI scheme:

- `db:attribute`
- `db:concept`
- `db:name`

The `db:` scheme is protected against assertions, nothing
the can change what lives at these URIs. This is what makes `db:concept!:`
a stable escape hatch even when a name like `concept` is reassigned to a
different entity.

By default names `attribute`, `concept`, `name` are assigned these entities, so
out-of-the-box `concept!:` works without qualification. The mapping is a regular
name assertion and can be re-asserted by user assertions; the built-in entities
themselves cannot.

### `attribute`

Defines an attribute by domain/name, value type, and
cardinality. Although `attribute` is a built-in, its is a regular concept and
can be described in the notation:

```yaml
# The four attributes that make up the attribute concept.

attribute!:
  this: ?id
  description: "The attribute selector in domain/name form"
  the:         dialog.attribute/id
  as:          text
  cardinality: one

attribute!:
  this: ?type
  description: "The value-type discriminant (text, unsigned-integer, …)"
  the:         dialog.attribute/type
  as:          text
  cardinality: one

attribute!:
  this: ?cardinality
  description: "Cardinality: one or many"
  the:         dialog.attribute/cardinality
  as:          text
  cardinality: one

attribute!:
  this: ?description
  description: "Human-readable description"
  the:         dialog.meta/description
  as:          text
  cardinality: one

# The attribute concept itself.

concept!: &attribute
  description: "An attribute definition"
  with:
    the:        ?id
    as:          ?type
    cardinality: ?cardinality
    description: ?description
```

The body field shorthands users write (`the:`, `as:`,
`cardinality:`, `description:`) map to the four attributes
above through the concept's `with:` map.

`description` is required on every attribute, but it does not
participate in the attribute entity's content-derivation —
two attributes with identical `the:`, `as:`, and
`cardinality:` but different `description:` claims resolve to
the same entity. Changing the description of an existing
attribute is a mutation on a stable entity, not the creation
of a new one.

Because `attribute` is a regular concept, you can query it:

```yaml
# Find every attribute whose cardinality is "many".
attribute:
  this:        ?a
  the:         ?selector
  cardinality: many
```

Note: the schema definition above is illustrative. In an
actual branch, these attributes and the `attribute` concept
itself are built-in.

### `concept`

Composes attributes into a shape with named fields:

```yaml
concept!: &person
  description: "A person"
  with:
    name: person-name
    age:  person-age
```

Fields:

- `with` (required) — required attributes of the concept.
  Each value is a bare symbol or a URI.
- `description` (optional but conventionally present) —
  surfaces in editor hover.

The `concept` concept itself is *not* expressible in this
notation, because its `with:` field is a dictionary (an
arbitrary map of names to attribute references) rather than a
fixed record of named fields. The notation can express
assertions *against* the `concept` concept, but cannot
express the `concept` concept's own schema.

### `name`

Establishes a name that can be resolved to an associated entity.

```yaml
name!:
  this:   id:alice
  entity: did:key:zAlice
```

Fields:

- `entity` - the entity currently identified by this name.
  Cardinality one on the underlying attribute, so each name
  has exactly one target at a time.

The schema, illustratively:

```yaml
attribute!:
  this: ?entity
  description: "The entity identified by the name"
  the:         dialog.meta/name
  as:          entity
  cardinality: one

concept!: &name
  description: "A mutable name for an entity"
  with:
    entity: ?entity
```

Because the underlying attribute is `dialog.meta/name`,
queries can find every name pointing at a given entity, or
every entity with at least one name, using the regular query
machinery:

```yaml
# Find all names for this entity.
name:
  this:   ?n
  entity: did:key:zHjKf…
```

In practice you rarely write `name!` directly; the `&`
anchor sugar covers most uses.

## Why the parser is permissive

The parser produces diagnostics as it goes and continues past
recoverable errors so the editor can show every problem in
one pass. A document with three malformed expressions
surfaces three diagnostics, not one.

This means `parse(code).syntax` can be `Some(..)` *with* a
non-empty `diagnostics` list — partial trees are
intentional, and the analyzer in `tonk-schema` will refuse to
build an `Analysis` from a tree that has any errors.
