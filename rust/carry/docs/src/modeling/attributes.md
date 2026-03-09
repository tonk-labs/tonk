# Attributes

An **attribute** is a relation elevated with type and cardinality constraints. Where a raw claim like `com.app.person/name` just associates a value with an entity, an attribute says: "this relation accepts `Text` values and each entity has exactly `one`."

Attributes are the building blocks of [concepts](./concepts.md).

## Defining Attributes

### Via the CLI

```bash
carry assert attribute @person-name \
  the=com.app.person/name \
  as=Text \
  cardinality=one \
  description="Name of a person"
```

The `@person-name` creates a bookmark so you can reference this attribute by name later.

### Via YAML

```yaml
person-name:
  attribute:
    description: Name of a person
    the: com.app.person/name
    as: Text
    cardinality: one
```

The top-level key (`person-name`) becomes the bookmark name.

## Attribute Fields

| Field | Required | Description |
|---|---|---|
| `the` | Yes | The relation identifier -- `domain/name` format |
| `as` | Yes | The value type (see below) |
| `cardinality` | No | `one` (default) or `many` |
| `description` | Yes | Human-readable description |

## Value Types

| Type | Description | Example Values |
|---|---|---|
| `Text` | UTF-8 string | `"Alice"`, `"hello world"` |
| `UnsignedInteger` | Non-negative integer | `0`, `28`, `1000` |
| `SignedInteger` | Signed integer | `-5`, `0`, `42` |
| `Float` | Floating-point number | `3.14`, `-0.5` |
| `Boolean` | True or false | `true`, `false` |
| `Symbol` | A namespaced constant | `carry.profile/work` |
| `Entity` | Reference to another entity | `did:key:z...` |
| `Bytes` | Raw binary data | *(binary)* |

You can also specify an **enumeration** of allowed symbols:

```yaml
task-status:
  attribute:
    description: Current status of a task
    the: com.app.task/status
    as: [":todo", ":in-progress", ":done"]
    cardinality: one
```

## Cardinality

- `one` -- Each entity has at most one value for this attribute. Asserting a new value replaces the old one.
- `many` -- Each entity can have multiple values. Asserting adds to the set; retracting a specific value removes it.

```yaml
# A person has one name
person-name:
  attribute:
    description: Name
    the: com.app.person/name
    as: Text
    cardinality: one

# A recipe can have many ingredients
recipe-ingredient:
  attribute:
    description: An ingredient in the recipe
    the: diy.cook/ingredient
    as: Entity
    cardinality: many
```

## Attribute Identity

Two attributes with the same relation identifier (`the`) but different type or cardinality are **distinct entities**. The attribute's DID is derived from the hash of `(relation_id, type, cardinality)`. The description does not affect identity.

This means you can have, for example, both a `Text` version and an `Entity` version of the same relation, and they will be treated as different attributes.

## Querying Attributes

```bash
# List all defined attributes
carry query attribute

# Find a specific attribute by name
carry query attribute the=com.app.person/name
```
