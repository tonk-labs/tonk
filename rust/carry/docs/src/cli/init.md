# carry init

Create a new Dialog DB repository.

## Synopsis

```
carry init [LABEL] [--repo <PATH>]
```

## Description

Creates a `.carry/` directory. If `--repo` is not specified, the repository is created in the current working directory.

The command:

1. Generates an Ed25519 keypair for the repository.
2. Creates `.carry/<did>/` with a `credentials` file and `claims/` directory.
3. Bootstraps the builtin concepts (`attribute`, `concept`, `bookmark`) so they can be used immediately.
4. If `LABEL` is provided, asserts it as the repository label.

If a `.carry/` directory already exists at the target location, the command reports its status.

## Arguments

| Argument | Description |
|---|---|
| `LABEL` | Optional label for the repository (e.g., "my-project") |

## Options

| Flag | Description |
|---|---|
| `--repo <PATH>` | Directory where `.carry/` should be created. Defaults to `$PWD`. |

## Examples

```bash
# Initialize in current directory
carry init

# Initialize with a label
carry init my-project

# Initialize in a specific directory
carry init --repo /path/to/project

# Initialize with label in specific directory
carry init my-project --repo /path/to/project
```

## Output

```
Initialized my-project repository in /path/to/.carry/did:key:zAbc123
```

## Notes

- Running `carry init` inside a directory that is already within an existing repository creates a **nested repository**. Carry does not detect or warn about nesting.
- The DID (e.g., `did:key:zAbc123`) is derived from the generated public key and is globally unique.
- The private key at `.carry/<did>/credentials` is stored with mode `0600` (owner read/write only).
