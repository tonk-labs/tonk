# carry init

Create a new Dialog DB repository.

## Synopsis

```
carry init [LABEL] [--site <PATH>]
```

## Description

Creates a `.carry/` directory containing a new space. If `--site` is not specified, the repository is created in the current working directory.

The command:

1. Generates an Ed25519 keypair for the space.
2. Creates `.carry/<space-did>/` with a `credentials` file and `facts/` directory.
3. Bootstraps the builtin concepts (`attribute`, `concept`, `bookmark`) so they can be used immediately.
4. If `LABEL` is provided, asserts it as the space label.

If a `.carry/` directory already exists at the target location, the command reports its status without creating another space. Use `carry space create` to add more spaces after initialization.

## Arguments

| Argument | Description |
|---|---|
| `LABEL` | Optional label for the space (e.g., "my-project") |

## Options

| Flag | Description |
|---|---|
| `--site <PATH>` | Directory where `.carry/` should be created. Defaults to `$PWD`. |

## Examples

```bash
# Initialize in current directory
carry init

# Initialize with a label
carry init my-project

# Initialize in a specific directory
carry init --site /path/to/project

# Initialize with label in specific directory
carry init my-project --site /path/to/project
```

## Output

```
Initialized my-project repository in /path/to/.carry/did:key:zAbc123
```

## Notes

- Running `carry init` inside a directory that is already within an existing repository creates a **nested repository**. Carry does not detect or warn about nesting.
- The space DID (e.g., `did:key:zAbc123`) is derived from the generated public key and is globally unique.
- The private key at `.carry/<did>/credentials` is stored with mode `0600` (owner read/write only).
