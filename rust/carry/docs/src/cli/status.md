# carry status

Display information about the current repository and space.

## Synopsis

```
carry status [--repo <PATH>] [--format <FMT>]
```

## Description

Shows the resolved `.carry/` repository path, the active space DID, and the space label (if set). Useful for verifying which space commands will operate on.

## Options

| Flag | Description |
|---|---|
| `--repo <PATH>` | Path to `.carry/` repository |
| `--format <FMT>` | Output format: `yaml` (default) or `json` |

## Examples

```bash
# Show status
carry status

# Show status as JSON
carry status --format json
```

## Output

```
Repo: /path/to/project/.carry
Spaces:
  did:key:zAbc123 (active)
  did:key:zDef456
```

With `--format json`:

```json
{
  "repo": "/path/to/project/.carry",
  "spaces": [
    {"did": "did:key:zAbc123", "active": true, "path": "/path/to/project/.carry/did:key:zAbc123"},
    {"did": "did:key:zDef456", "active": false, "path": "/path/to/project/.carry/did:key:zDef456"}
  ]
}
```
