# carry retract

Retract claims from entities -- remove data.

## Synopsis

```
carry retract <TARGET|FILE|-> [this=<ENTITY>] [FIELD[=VALUE] ...] [--repo <PATH>] [--format <FMT>]
```

## Description

Retract removes claims from the indexes. Retracted claims no longer appear in query results.

### Input Modes

| Mode | Syntax | Description |
|---|---|---|
| Target | `carry retract <domain-or-concept> this=<entity> field ...` | Retract from CLI arguments |
| File | `carry retract <file.yaml>` | Retract from a YAML or JSON file |
| Stdin | `carry retract -` | Retract from standard input |

## Arguments

| Argument | Description |
|---|---|
| `TARGET` | Domain (e.g., `com.app.person`) or concept name (e.g., `person`) |
| `FILE` | Path to a YAML or JSON file |
| `-` | Read from stdin |

## Fields

| Syntax | Description |
|---|---|
| `field` | Retract this field regardless of its current value |
| `field=value` | Retract only if the claim matches this exact value |
| `this=<DID>` | The entity to retract from (required for target mode) |

Using `field=value` is useful for `cardinality: many` attributes where an entity has multiple values and you only want to remove one.

## Options

| Flag | Description |
|---|---|
| `--repo <PATH>` | Path to `.carry/` repository |
| `--format <FMT>` | Output format |

## Examples

```bash
# Retract a field (any value)
carry retract person this=did:key:zAlice age

# Retract a specific value (for multi-valued fields)
carry retract person this=did:key:zAlice tag=urgent

# Retract using a domain
carry retract com.app.person this=did:key:zAlice name age

# Retract all claims on an entity
carry retract com.app.person this=did:key:zAlice

# Retract from a file
carry retract retractions.yaml

# Retract from stdin (pipe query output to remove matching data)
carry query person name="Alice" --format triples | carry retract -
```

## Notes

- Unlike `assert`, target-mode `retract` requires `this=` to identify which entity to modify.
- Other claims on the entity are unaffected -- only the specified fields are retracted.
- Retracted claims are removed from the indexes and no longer appear in query results.
- If no fields are specified with `this=`, **all** claims on that entity are retracted.
