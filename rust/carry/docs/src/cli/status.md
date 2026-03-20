# carry status

Display information about the current repository.

## Synopsis

```
carry status [--repo <PATH>] [--format <FMT>]
```

## Description

Shows the resolved `.carry/` repository path and DID.

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
DID: did:key:zAbc123
```

With `--format json`:

```json
{
  "repo": "/path/to/project/.carry",
  "did": "did:key:zAbc123"
}
```
