# Asserted-notation guide

A YAML-flavoured DSL for reading and writing facts about entities
in a tonk repository. One document is a sequence of expressions;
each expression is a query, an assertion, or a retraction.

## At a glance

```yaml
# Query: find every person.
person:

# Query with field constraints + variables.
person ?alice:
  name: "Alice"
  age:  ?age

# Query a claim (reverse-domain head); fields are explicit.
xyz.tonk ?tonkee:
  name: "Alice"
  role: ?role

# Join across expressions on shared variables (?alice here).
person ?alice:
  name: "Alice"
xyz.tonk:
  person: ?alice
  role:   ?role

# Assertion: declare facts about a fresh person entity.
person!:
  name: "Nick"
  address: "Portland, OR"

# Update: add facts to an existing entity (other fields unchanged).
person! did:key:zNick:
  name: "Nicholas"

# Retract a concept-projection from an entity.
person! ?nick: _

# Field-level retraction: drop just one attribute.
person! ?nick:
  address: _

# Bookmark an entity by name for easy later reference.
person! nick:
  name: "Nick"
  address: "Portland, OR"

# Define an attribute (built-in `attribute` concept).
attribute! person-name:
  description: "The person's name"
  the:         io.gozala.person/name
  as:          Text
  cardinality: one

# Define a concept (built-in `concept` concept).
concept! person:
  description: "A person"
  with:
    name: .person-name
    age:  .person-age
```

## Three flavours of expression

Every top-level entry in a document is one of three things,
distinguished by the `!` suffix on the head and the body shape:

| Head form         | Body         | Meaning      |
|-------------------|--------------|--------------|
| `head:`           | fields or empty | **Query**    |
| `head!:`          | fields       | **Assertion** |
| `head!:`          | `_`          | **Retraction** |

A query body cannot be `_`. A non-`!` head with a `_` body is a
parse error.

## Heads

A head is a name plus an optional binding:

```
<name>[!] [<binding>]
```

### Name

- A bare identifier (`person`, `attribute`, `concept`) is a
  **concept** name. The analyzer resolves it via the branch — the
  built-in concepts `attribute` and `concept` always resolve;
  user-defined concepts must have been previously asserted.
- A reverse-dotted identifier (`xyz.tonk`, `io.gozala.person`) is a
  **claim** domain. Each field name combines with the domain to
  form an attribute URI (`xyz.tonk/role`).

### `!` — effect marker

The trailing `!` marks the expression as having an effect. Without
it, the expression is a query (a read, no transaction). With it,
the expression contributes to a transaction (assertion or
retraction).

### Binding

Whitespace-separated from the name. Identifies *which* entity the
expression refers to.

| Binding             | Meaning                                                |
|---------------------|--------------------------------------------------------|
| (omitted)           | Anonymous — query: any entity; assertion: a fresh entity |
| `?var`              | Bind / refer to entity as variable named `var`         |
| `.bookmark` *(query)* | Refer to the entity previously bookmarked under this name |
| `bookmark` *(assertion)* | Derive an entity from the bookmark name and assert a name binding |
| `did:key:zX`        | Explicit entity URI                                    |

Variables join across expressions in the same document — see
**Joins** below.

## Bodies

A body is one of:

- **A mapping of fields** — most expressions.
- **`_` (a single underscore)** — only on `head!:`. Concept
  retraction (drops every fact for the entity's
  concept-projection) or claim retraction (drops every fact
  on the entity whose attribute belongs to the claim's domain).
- **Empty** — `head:` with no fields. Equivalent to "match any
  entity satisfying the head" (queries) or a no-op (assertions,
  rejected by the analyzer as useless).

## Field values

Fields go on the right of `field:`. Five flavours, distinguished
lexically by the parser:

| Source       | Meaning                                                  |
|--------------|----------------------------------------------------------|
| `"Alice"` or `Alice` | String literal                                  |
| `28`, `1.5`, `true`, `null` | Primitive literal                        |
| `?name`      | Logic variable                                           |
| `_`          | Blank — query: match any value; assertion: retract this attribute |
| `.bookmark`  | Bookmark reference                                       |
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

### Nested mappings

Field values can be nested mappings — e.g. `concept!`'s `with:`
block, or an inline `attribute!` definition inside a `with:`:

```yaml
concept! person:
  with:
    name:
      attribute!:
        the:         io.gozala.person/name
        as:          Text
        cardinality: one
```

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

Variables in an assertion or retraction body must be bound by
some query expression in the same document — the analyzer
diagnoses unbound references.

## Blanks (`_`)

Three contexts, three meanings:

1. **Field value in a query** — match any value. The matched
   value isn't surfaced as a join key; if you want to refer to it
   later, use a named variable (`?x`).
2. **Field value in an assertion** — retract just this attribute
   for the head's entity.
3. **Whole body** — concept-level retraction (concept heads) or
   domain-level retraction (claim heads).

## Bookmarks

Bookmarks are named handles for entities. They live in the
repository, asserted via the `dialog.meta/name` claim under the
hood.

### Defining a bookmark

Use a bookmark binding on an assertion. The entity is derived
deterministically from the bookmark name (blake3 of the name into
a `did:key:z…`), and a name binding is asserted alongside the
content:

```yaml
person! nick:
  name: "Nick"
  address: "Portland, OR"
```

After this transaction, the entity for `nick` is fixed; future
references to `.nick` resolve to the same `did:key:z…`.

### Referring to a bookmark

In **field-value position**, prefix with `.`:

```yaml
person ?p:
  best-friend: .nick

concept! person:
  with:
    name: .person-name
```

In **head-binding position** (queries), use `.bookmark`:

```yaml
person .nick:
  name: ?name
```

In **head-binding position** (assertions), the bookmark name
appears bare — the `!` already marks the expression as an
assertion, and the bare name in binding position is unambiguous:

```yaml
person! nick:
  name: "Nick"
```

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

For now, queries are limited to a single document scope — there
are no cross-document references.

## Built-in concepts

Two concepts are always resolvable: `attribute` and `concept`.
They define the schema of attributes and concepts themselves.

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
- `as` (required) — the value type (`Text`, `UnsignedInteger`,
  `Boolean`, `Entity`, etc.).
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
  maybe:
    nickname: .person-nickname
```

Fields:

- `with` (required) — required attributes of the concept. Each
  value is a bookmark reference or URI to an attribute.
- `maybe` (optional) — optional attributes.
- `description` (optional) — surfaces in editor hover and
  completion.

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
