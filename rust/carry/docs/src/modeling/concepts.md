# Concepts

A **concept** is a composition of [attributes](./attributes.md) that describes the shape of a thing. Think of it as a lightweight, named schema -- like a type or class, but realized through schema-on-read rather than schema-on-write.

Concepts are the primary unit of domain modeling in Carry.

## Defining Concepts

### Via the CLI

First define the attributes, then compose them into a concept:

```bash
# Define attributes
carry assert attribute @person-name \
  the=com.app.person/name as=Text cardinality=one \
  description="Name of a person"

carry assert attribute @person-age \
  the=com.app.person/age as=UnsignedInteger cardinality=one \
  description="Age of a person"

# Compose into a concept
carry assert concept @person \
  description="A person" \
  with.name=person-name \
  with.age=person-age
```

The `with.name=person-name` means: "the field called `name` in this concept uses the `person-name` attribute." The left side of `=` is the field name; the right side is the attribute bookmark.

### Via YAML (Separate Definitions)

```yaml
person-name:
  attribute:
    description: Name of a person
    the: com.app.person/name
    as: Text
    cardinality: one

person-age:
  attribute:
    description: Age of a person
    the: com.app.person/age
    as: UnsignedInteger
    cardinality: one

person:
  concept:
    description: A person
    with:
      name: person-name
      age: person-age
```

### Via YAML (Inline Attributes)

For convenience, you can define attributes inline within a concept:

```yaml
person:
  concept:
    description: A person
    with:
      name:
        description: Name of a person
        the: com.app.person/name
        as: Text
        cardinality: one
      age:
        description: Age of a person
        the: com.app.person/age
        as: UnsignedInteger
        cardinality: one
```

Both forms produce identical claims.

## Required vs. Optional Fields

Concepts distinguish between **required** fields (`with`) and **optional** fields (`maybe`):

```yaml
task:
  concept:
    description: A task to be completed
    with:
      title:
        description: Title of the task
        the: com.app.task/title
        as: Text
      status:
        description: Current status
        the: com.app.task/status
        as: [":todo", ":in-progress", ":done"]
    maybe:
      priority:
        description: Priority level
        the: com.app.task/priority
        as: [":low", ":medium", ":high"]
```

An entity matches a concept if all `with` fields are present, regardless of which `maybe` fields exist. Optional fields are included in query output when present but don't affect concept membership.

## Concept Identity

A concept's identity is derived from its complete set of required fields -- the `(field_name, attribute_entity)` pairs. Both the field names and the attributes they point to participate in identity:

- A concept with a field named `name` pointing at attribute `A` is **distinct** from one with a field named `fullname` pointing at the same attribute `A`.
- Optional fields (`maybe`) do **not** participate in concept identity.

## Using Concepts

### Assert Data Against a Concept

```bash
carry assert person name=Alice age=28
```

Fields are validated against the concept's schema. If a required field is missing or a value doesn't match the expected type, the assertion is rejected.

### Query by Concept

```bash
carry query person
carry query person name="Alice"
```

Concept queries return all fields defined by the concept (both `with` and `maybe`), unlike domain queries where you must explicitly request each field.

### Concept Queries vs. Domain Queries

| | Domain Query | Concept Query |
|---|---|---|
| Target | `com.app.person` (contains `.`) | `person` (no `.`) |
| Fields returned | Only those you request | All fields the concept defines |
| Schema validation | None | Validated against concept |
| Requires schema | No | Yes (concept must be defined) |

## Referencing Attributes

In the CLI, `with.<field>=<value>` can reference attributes in two ways:

- **By bookmark name**: `with.name=person-name` -- looks up the attribute by its `dialog.meta/name`.
- **By selector**: `with.name=com.app.person/name` -- looks up (or auto-creates) the attribute by its relation identifier. If the value contains `/`, it's treated as a selector.

## Querying Concept Definitions

```bash
# List all defined concepts
carry query concept

# Show a specific concept
carry query concept description
```
