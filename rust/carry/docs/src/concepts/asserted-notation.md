# Asserted Notation

Asserted notation is the canonical YAML format that Carry uses for command output and file-based input. It represents data as a three-level hierarchy that expands unambiguously to a set of raw claims.

Because `carry query` output and `carry assert -` input share the same format, you can pipe query output directly back as input without any transformation.

## Three-Level Structure

Every entry in asserted notation follows:

```yaml
<entity-identifier>:
  <context>:
    <field>: <value>
```

### Level 1: Entity Identifier

The outermost key identifies the entity being described.

| Form | Meaning | Example |
|---|---|---|
| Contains `:` | Global identifier (a DID or URI) | `did:key:zAlice` |
| No `:` | Local bookmark name | `quantity`, `person` |

### Level 2: Context

The second key declares how the fields beneath it should be interpreted.

| Form | Meaning | Example |
|---|---|---|
| Contains `.` | Domain context -- fields expand to `domain/field` relation identifiers | `com.app.person` |
| No `.` | Concept context -- fields are named attributes of that concept | `attribute`, `concept`, `bookmark` |

### Level 3: Fields

Named values within the context.

- **Scalar value**: A direct association -- `name: Alice`, `age: 28`
- **Non-scalar value** (a nested YAML map): Implies a nested entity

## Data Assertions (Domain Context)

Under a domain context, each field expands to a claim:

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28
```

Expands to:

```yaml
- the: com.app.person/name
  of:  did:key:zAlice
  is:  Alice

- the: com.app.person/age
  of:  did:key:zAlice
  is:  28
```

## Schema Definitions (Concept Context)

Under a concept context, the pre-registered concept schema determines how fields are interpreted. See [Attributes](../modeling/attributes.md) and [Concepts](../modeling/concepts.md) for details.

```yaml
person-name:
  attribute:
    description: The person's name
    the: com.app.person/name
    as: Text
    cardinality: one
```

## Anonymous Entities

Use `_` as the entity identifier when you don't care about the identity:

```yaml
_:
  diy.cook:
    quantity: 2
    ingredient: carrot
```

Each `_` creates a fresh entity. If you need to reference the same anonymous entity in multiple places within one document, use a named variable like `?foo`:

```yaml
?meal:
  diy.planner:
    attendee: ?person
    recipe: ?recipe
```

All occurrences of `?foo` in the same document bind to the same generated entity.

## Nested Entities

A non-scalar value under a domain context implies a nested entity:

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    address:
      city: San Francisco
      zip: 94107
```

Expands to:

```yaml
- the: com.app.person/name
  of:  did:key:zAlice
  is:  Alice

- the: com.app.person/address
  of:  did:key:zAlice
  is:  <address-entity>

- the: com.app.address/city
  of:  <address-entity>
  is:  San Francisco

- the: com.app.address/zip
  of:  <address-entity>
  is:  94107
```

The nested entity's domain is derived from the parent domain with the field name appended as a segment.

## EAV Triple Format

For piping between commands, Carry also supports a flat triple format via `--format triples`:

```yaml
- the: com.app.person/name
  of: did:key:zAlice
  is: Alice
- the: com.app.person/age
  of: did:key:zAlice
  is: 28
```

This format is accepted by both `carry assert -` and `carry retract -`.

## JSON

JSON is supported as a structural equivalent to YAML. The same three-level hierarchy applies:

```json
{
  "did:key:zAlice": {
    "com.app.person": {
      "name": "Alice",
      "age": 28
    }
  }
}
```

EAV triples in JSON:

```json
[
  {"the": "com.app.person/name", "of": "did:key:zAlice", "is": "Alice"},
  {"the": "com.app.person/age", "of": "did:key:zAlice", "is": 28}
]
```

## Round-Trip Property

Because query output is asserted notation and assert accepts it, the following always works:

```bash
carry query person name="Alice" | carry assert -
carry query person --format triples | carry retract -
```

This makes Carry composable in the Unix tradition: commands produce output that other commands can consume.
