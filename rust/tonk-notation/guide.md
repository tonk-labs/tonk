# Asserted-notation guide

A YAML-flavoured DSL for reading and writing facts about entities
in a tonk repository. One document is a sequence of expressions;
each expression is a query, an assertion, or a retraction. The
worker's `/evaluate` route runs a whole document in one
transaction, returning matches and a commit summary.

This guide builds up examples that you can paste into the editor
in order — each one runs against the branch the previous ones
left behind.

## Three flavours of expression

Every top-level entry in a document is one of three things,
distinguished by the `!` suffix on the head and the body shape:

| Head form | Body            | Meaning       |
|-----------|-----------------|---------------|
| `head:`   | fields or empty | **Query**     |
| `head!:`  | fields          | **Assertion** |
| `head!:`  | `_`             | **Retraction** |

A query body cannot be `_`. A non-`!` head with a `_` body is a
parse error.

## Worked tutorial

The shortest path from "empty branch" to "you can read and write
your own data." Run the snippets in order.

### 1. Define an attribute

`attribute!` is the built-in head for declaring an attribute. The
body says where the attribute lives (`the:`), what kind of value
it carries (`as:`), and how many values per entity (`cardinality:`).
The bookmark name (`person-name`) writes a `dialog.meta/name`
claim so future documents can reference it as `.person-name`.

```yaml
attribute! person-name:
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
  description: "The person's name"
```

After the commit, `.person-name` resolves to a content-derived
attribute entity. The bookmark name persists on the branch.

### 2. Define a few more attributes

Attributes are independent — each `attribute!` head produces one
attribute entity.

```yaml
attribute! person-age:
  the:         io.gozala.person/age
  as:          UnsignedInteger
  cardinality: one
  description: "The person's age in years"
```

You can put multiple `attribute!` heads in the same document; they
all commit in one transaction. The next step does that.

### 3. Define a concept (and its attributes in one shot)

`concept!` composes attributes into a named shape. Its `with:`
block maps field names to attribute references. References use
`.bookmark` (a name lookup) or a `the:…` URI.

```yaml
attribute! person-name:
  description: name of the person
  the:         xyz.tonk.person/name
  as:          Text
  cardinality: one

attribute! person-age:
  description: age of the person
  the:         xyz.tonk.person/age
  as:          UnsignedInteger
  cardinality: one

concept! person:
  description: "A person"
  with:
    name: .person-name
    age:  .person-age
```

`.person-name` resolves against the in-document attributes first
(those two `attribute!` heads above), so a one-shot schema commit
works without a separate prior commit.

### 4. Add a person

`person! …:` is an assertion against the `person` concept defined
above. The `alice` is a bookmark — git-tag semantics:

- The entity is derived from the body's content
  (`Entity::of(&{name: "Alice", age: 28})`).
- A `dialog.meta/name = "alice"` claim points the bookmark name
  at that entity.
- Re-running the same body is a no-op (same entity, same claim).
- Re-running with a *different* body produces a different entity,
  and `.alice` rebinds to the new one (cardinality-one on
  `dialog.meta/name` retracts the old binding).

```yaml
person! alice:
  name: "Alice"
  age:  28
```

### 5. Read it back

Anonymous query — `person:` matches every entity satisfying the
`person` concept's schema, surfaces every field:

```yaml
person:
```

Result (one block per source query, one entry per match):

```
person
  did:key:zHjKf…           # alice's entity
    name: "Alice"
    age:  28
```

Constrain the match by giving fields literal values:

```yaml
person:
  name: "Alice"
```

Bind a field to a logic variable to see it in the response:

```yaml
person ?p:
  name: "Alice"
  age:  ?age
```

`?p` and `?age` come back in the result frame.

### 6. Update Alice

Two ways. The "git tag" way — same bookmark, different body:

```yaml
person! alice:
  name: "Alice"
  age:  29
```

This produces a *new* entity (different body hash) and rebinds
`.alice`. Alice's old entity still has the old name+age claims;
only the bookmark moved. If you want to update *the same* entity,
use a query-bound variable.

The "find-and-update" way — query first, then assert against the
matched entity:

```yaml
person ?alice:
  name: "Alice"
person! ?alice:
  age:  30
```

The query binds `?alice` to every Alice; the assertion writes
`age = 30` on each match. `dialog.meta/age` is `cardinality: one`,
so the prior age claim is auto-retracted.

### 7. Retract Alice's whole projection

Concept-level retraction uses a `_` body. The entity has to come
from a URI or a query binding (anonymous retraction would have
nothing to act on).

```yaml
person ?alice:
  name: "Alice"
person! ?alice: _
```

The worker queries the branch for every fact whose attribute is
in the `person` concept's `with` map and whose subject is the
matched `?alice` entity, and dissociates each match.

By URI directly:

```yaml
person! did:key:zHjKf…: _
```

(Substitute Alice's actual entity URI from the response.)

## Heads in detail

A head is a name plus an optional binding:

```
<name>[!] [<binding>]
```

### Name

- A bare identifier (`person`, `attribute`, `concept`) is a
  **concept** name. The analyzer resolves it via the branch — the
  built-in concepts `attribute` and `concept` are always
  resolvable; user-defined concepts must have been previously
  asserted (or defined earlier in the same document).
- A reverse-dotted identifier (`xyz.tonk`, `io.gozala.person`) is
  a **claim** domain. Each field name combines with the domain to
  form an attribute URI (`xyz.tonk/role`). Claim heads have no
  schema, so the body must enumerate every field you care about
  — `xyz.tonk:` with no body is a parse error.

### `!` — effect marker

The trailing `!` marks the expression as having an effect.
Without it, the expression is a query (a read, no transaction).
With it, the expression contributes to a transaction (assertion
or retraction).

### Binding

Whitespace-separated from the name. Identifies *which* entity the
expression refers to.

| Binding             | Query side                                    | Assertion side                                   |
|---------------------|-----------------------------------------------|--------------------------------------------------|
| (omitted)           | `this` is a free variable; matches any entity | Body-derived entity; no name claim               |
| `?var`              | Bind matches to `?var` (joins across exprs)   | Bound by query if any expr binds it; otherwise body-derived, registered as `?var` for later expressions in this doc |
| `bookmark`          | (not currently a head-side query form)        | Body-derived entity, plus `dialog.meta/name = bookmark` claim (git-tag semantics) |
| `did:key:zX`        | Match exactly that entity                     | Use that entity verbatim                         |

Variables join across expressions in the same document — see
**Joins** below.

### Bookmarks vs. variables on `attribute!` / `concept!`

`attribute!` and `concept!` derive their entity from the
descriptor (the body), not from `Entity::of(&body)` — they're
content-addressed by the schema they declare, so two documents
defining the same shape land on the same entity. The bookmark vs
variable distinction is just whether to write a `dialog.meta/name`
claim:

- `attribute! foo:` writes `dialog.meta/name = "foo"`. Future
  documents can reference `.foo`.
- `attribute! ?foo:` writes no name claim. `?foo` is visible to
  later expressions in the same document only.

Use a bookmark when you want a stable name on the branch. Use a
variable when you just need to reference the attribute once,
locally.

## Bodies

A body is one of:

- **A mapping of fields** — most expressions.
- **`_` (a single underscore)** — only on `head!:`. Concept
  retraction (drops every fact for the entity's concept-projection)
  or claim retraction (drops every fact on the entity whose
  attribute belongs to the claim's domain — currently
  unimplemented for claim heads; surfaces an analyzer error).
- **Empty** — `head:` with no fields. Anonymous query that
  surfaces every field of the concept's schema.

## Field values

Fields go on the right of `field:`. Five flavours, distinguished
lexically by the parser:

| Source       | Meaning                                                  |
|--------------|----------------------------------------------------------|
| `"Alice"` or `Alice` | String literal                                  |
| `28`, `1.5`, `true` | Primitive literal                                 |
| `?name`      | Logic variable                                           |
| `_`          | Blank — query: match any value                           |
| `.bookmark`  | Reference to a previously-bookmarked entity              |
| `did:key:…` (any text containing `:`) | URI reference                  |

Bare identifiers (no quotes, no leading sigil) are **literal
strings**, not bookmark references. References require an
explicit `.` prefix:

```yaml
# Literal string "person-name":
field: person-name

# Reference to the entity bookmarked as `person-name`:
field: .person-name
```

The leading `.` is unambiguous because no bare identifier begins
with one.

### Bookmark resolution in field position

`.bookmark` resolves in this order:

1. Bookmarks declared by an earlier head in the *same document*
   (`analysis.declarations`).
2. The branch's `dialog.meta/name` index — currently this only
   resolves attributes (via `Resolver::resolve_attribute`),
   because reading non-attribute names by bookmark from the
   branch isn't wired through the analyzer's sync resolver path
   yet.

So `.alice` (where `alice` was declared by an earlier `person! alice:`
in the same document) works; `.alice` against the branch alone
(`alice` was committed in a previous document) does not yet
resolve in field position. Use the URI form for now:
`field: did:key:zHjKf…`.

## Variables

`?name` is a logic variable. Two rules:

1. **In a query**, it binds whatever value matches the position.
2. **Across expressions in one document**, occurrences of the
   *same* variable name unify — i.e. the same value must satisfy
   every position the variable appears in.

```yaml
# Find people whose name matches their xyz.tonk role.
person ?p:
  name: ?n
xyz.tonk:
  person: ?p
  role:   ?n
```

Variables in an assertion or retraction body that aren't bound by
some query expression in the same document are accepted on the
*head* (where they introduce a new body-derived entity registered
under the variable name) but rejected in *field positions* (where
the analyzer can't infer a value).

## Blanks (`_`)

Three contexts, three meanings:

1. **Field value in a query** — match any value. The matched
   value isn't surfaced as a join key; if you want to refer to it
   later, use a named variable (`?x`).
2. **Whole body on a `head!:`** — concept-level retraction: drop
   every fact in the concept's `with` map for the head's entity.
3. **Field value in an assertion** — *currently a no-op*. The
   analyzer accepts `address: _` syntactically but the emitter
   skips blank terms on assert. Field-level retraction via this
   shape is a planned follow-up; for now use concept-level
   retraction (`person! …: _`) and re-assert what you want to
   keep.

## Joins

A document may contain multiple query expressions. Variables with
the same name across expressions join. Results are filtered to
the bindings that satisfy *every* expression simultaneously.

```yaml
# Find every person who is also someone's xyz.tonk contact,
# returning the entity, person name, and role.
person ?e:
  name: ?name
xyz.tonk:
  person: ?e
  role:   ?role
```

A query+mutation document also joins via shared variables:

```yaml
person ?alice:
  name: "Alice"
person! ?alice:
  age:  30
```

For each match of `?alice` from the query, the assertion fires
once with that entity bound.

For now, joins are limited to a single document scope — there are
no cross-document references.

## Built-in concepts

Two concepts are always resolvable: `attribute` and `concept`.
They define the schema of attributes and concepts themselves.
They're real concepts whose fields are real EAV attributes
(`dialog.attribute/id`, `dialog.attribute/type`,
`dialog.attribute/cardinality`, `dialog.meta/description`,
`dialog.meta/name`); the analyzer treats them like any other
concept.

### `attribute`

Defines an attribute by domain/name, value type, and cardinality.

```yaml
attribute! person-name:
  description: "The person's name"
  the:         io.gozala.person/name
  as:          Text
  cardinality: one
```

Fields:

- `the` (required) — the attribute URI in `domain/name` form.
- `as` (optional) — the value type (`Text`, `UnsignedInteger`,
  `Boolean`, `Entity`, etc.). Defaults to no type constraint.
- `cardinality` (optional) — `one` (default) or `many`.
- `description` (optional) — human-readable description; surfaces
  in editor hover.

### `concept`

Composes attributes into a shape with named fields.

```yaml
concept! person:
  description: "A person"
  with:
    name: .person-name
    age:  .person-age
```

Fields:

- `with` (required) — required attributes of the concept. Each
  value is a `.bookmark` reference or a `the:…` URI to an
  attribute.
- `description` (optional) — surfaces in editor hover and
  completion.

`maybe:` (optional attributes) is part of dialog's
`ConceptDescriptor` model but not yet wired into the analyzer —
a `concept!` body containing `maybe:` returns
`UnknownField { concept: "concept", field: "maybe" }`.

Once defined, the concept name is itself a head:

```yaml
person! alice:
  name: "Alice"
  age:  28
```

## Why the parser is permissive

The parser produces diagnostics as it goes and continues past
recoverable errors so the editor can show every problem in one
pass. A document with three malformed expressions surfaces three
diagnostics, not one.

This means `parse(text).syntax` can be `Some(..)` *with* a
non-empty `diagnostics` list — partial trees are intentional, and
the analyzer in `tonk-schema` will refuse to build an `Analysis`
from a tree that has any errors.
