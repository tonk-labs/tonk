# carry query

Query entities by domain or concept.

## Synopsis

```
carry query <TARGET> [FIELD[=VALUE] ...] [--repo <PATH>] [--format <FMT>]
```

## Description

Query returns matching entities in [asserted notation](../concepts/asserted-notation.md). The target determines the kind of query:

- **Domain query** (target contains `.`): Searches for entities with claims in that domain. You choose which fields to include in output.
- **Concept query** (target has no `.`): Resolves the named concept via bookmark, returns all fields the concept defines.

## Arguments

| Argument | Description |
|---|---|
| `TARGET` | Domain (e.g., `com.app.person`) or concept name (e.g., `person`) |

## Fields

| Syntax | Description |
|---|---|
| `name` | Projection -- include this field in output |
| `name="Alice"` | Filter -- only return entities where name matches this value |

Filter fields narrow results. Projection fields expand what's shown. For concept queries, all concept fields are always included in output; specify fields only to filter.

## Options

| Flag | Description |
|---|---|
| `--repo <PATH>` | Path to `.carry/` repository |
| `--format <FMT>` | Output format: `yaml` (default), `json`, or `triples` |

## Examples

### Domain Queries

```bash
# Get name and age for all entities in the domain
carry query com.app.person name age

# Filter: only entities where name is Alice
carry query com.app.person name="Alice" age
```

### Concept Queries

```bash
# Get all fields of the 'person' concept
carry query person

# Filter by field value
carry query person name="Alice"
```

### Piping

```bash
# Pipe to assert (copy data)
carry query person --format triples | carry assert -

# Pipe to retract (remove matching data)
carry query person name="Alice" --format triples | carry retract -

# Asserted notation also pipes correctly
carry query com.app.person name age | carry assert -
```

### Querying Schema

```bash
# List all defined attributes
carry query attribute

# List all defined concepts
carry query concept
```

## Output Formats

### YAML (default)

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28

did:key:zBob:
  com.app.person:
    name: Bob
    age: 35
```

### JSON (`--format json`)

```json
[{"id": "did:key:zAlice", "name": "Alice", "age": 28}]
```

### Triples (`--format triples`)

```yaml
- the: com.app.person/name
  of: did:key:zAlice
  is: Alice
- the: com.app.person/age
  of: did:key:zAlice
  is: 28
```

## Notes

- Domain queries require at least one field to be specified (projection or filter).
- Concept queries with no fields return all entities matching the concept with all of the concept's fields.
