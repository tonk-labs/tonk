# carry assert

Assert claims on entities -- add or update data.

## Synopsis

```
carry assert <TARGET|FILE|-> [this=<ENTITY>] [@name] [FIELD=VALUE ...] [--repo <PATH>] [--format <FMT>]
```

## Description

Assert creates or updates claims. Claims are facts stored as `(the: relation, of: entity, is: value)`.

### Input Modes

| Mode | Syntax | Description |
|---|---|---|
| Target | `carry assert <domain-or-concept> field=value ...` | Assert from CLI arguments |
| File | `carry assert <file.yaml>` | Assert from a YAML or JSON file |
| Stdin | `carry assert -` | Assert from standard input |

### Target Detection

- `-` is always stdin
- Contains `/` or ends in `.yaml`, `.yml`, `.json` -- file path
- Contains `.` -- domain target
- Otherwise -- concept target (resolved by bookmark name)

## Arguments

| Argument | Description |
|---|---|
| `TARGET` | Domain (e.g., `com.app.person`) or concept name (e.g., `person`) |
| `FILE` | Path to a YAML or JSON file |
| `-` | Read from stdin |

## Fields

| Syntax | Description |
|---|---|
| `field=value` | Assert this field with this value |
| `this=<DID>` | Target an existing entity instead of creating a new one |
| `@name` | Assert `dialog.meta/name` on the entity (creates a bookmark) |
| `with.field=attr` | (Concept assertions) Required field referencing an attribute |
| `maybe.field=attr` | (Concept assertions) Optional field referencing an attribute |

## Options

| Flag | Description |
|---|---|
| `--repo <PATH>` | Path to `.carry/` repository |
| `--format <FMT>` | Output format: `yaml`, `json`, or `triples` |

## Examples

### Domain Assertions

```bash
# Create a new entity (DID printed to stdout)
carry assert com.app.person name=Alice age=28

# Update an existing entity
carry assert com.app.person this=did:key:zAlice age=29
```

### Concept Assertions

```bash
# Assert using a defined concept
carry assert person name=Alice age=28
```

### Builtin Concepts

```bash
# Define an attribute with a bookmark name
carry assert attribute @person-name \
  the=com.app.person/name as=Text cardinality=one \
  description="Name of a person"

# Define a concept
carry assert concept @person \
  description="A person" \
  with.name=person-name \
  with.age=person-age

# Create a bookmark
carry assert bookmark this=did:key:zEntity name=my-entity
```

### File and Stdin

```bash
# Assert from a YAML file
carry assert schema.yaml

# Assert from stdin
carry query person --format triples | carry assert -

# Assert from stdin (asserted notation also works)
carry query person | carry assert -
```

## File Formats

Assert accepts two YAML formats (auto-detected):

**Asserted notation** (from default `--format yaml`):

```yaml
did:key:zAlice:
  com.app.person:
    name: Alice
    age: 28
```

**EAV triples** (from `--format triples`):

```yaml
- the: com.app.person/name
  of: did:key:zAlice
  is: Alice
- the: com.app.person/age
  of: did:key:zAlice
  is: 28
```

**JSON EAV triples** are also accepted:

```json
[{"the": "com.app.person/name", "of": "did:key:zAlice", "is": "Alice"}]
```

## Output

When creating a new entity (no `this=`), the generated entity DID is printed to stdout:

```
did:key:zNewEntity123
```

## Notes

- Without `this=`, a new entity is created with a deterministic DID derived from the content.
- With `this=`, at least one field is required.
- The `@name` syntax is shorthand for asserting `dialog.meta/name` on the entity.
- For concept assertions, if a `with`/`maybe` value contains `/`, it's treated as an attribute selector (the attribute is looked up or auto-created). Without `/`, it's treated as an attribute bookmark name.
- Cardinality defaults to `one` for attribute assertions if not specified.
