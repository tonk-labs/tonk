# carry space

Manage spaces within a `.carry/` repository.

## Synopsis

```
carry space <SUBCOMMAND> [--site <PATH>] [--format <FMT>]
```

## Subcommands

| Subcommand | Alias | Description |
|---|---|---|
| `list` | `l` | List all spaces with labels |
| `create [LABEL]` | `c` | Create a new space and switch to it |
| `switch <DID\|LABEL>` | `s` | Switch the active space |
| `active` | `a` | Show the current active space |
| `delete <DID\|LABEL> [--yes]` | `d` | Delete a space |

## carry space list

List all spaces in the repository, marking the active space with `*`.

```bash
carry space list
```

Output:
```
* did:key:zAbc123  my-project
  did:key:zDef456  research
```

With `--format json`:
```json
[
  {"did": "did:key:zAbc123", "label": "my-project", "active": true},
  {"did": "did:key:zDef456", "label": "research", "active": false}
]
```

## carry space create

Create a new space and switch to it.

```bash
carry space create research
```

The optional label argument sets a human-readable name for the space. The new space gets its own Ed25519 keypair and becomes the active space.

## carry space switch

Switch the active space.

```bash
# By label
carry space switch research

# By DID
carry space switch did:key:zDef456
```

If a label matches multiple spaces, an error is returned -- use the DID to be specific.

The command is idempotent: switching to the already-active space is a no-op.

## carry space active

Show the current active space.

```bash
carry space active
```

## carry space delete

Delete a space from the repository.

```bash
carry space delete research
carry space delete research --yes    # skip confirmation prompt
```

| Flag | Description |
|---|---|
| `--yes` / `-y` | Skip the confirmation prompt |

You **cannot delete the active space**. Switch to a different space first:

```bash
carry space switch main
carry space delete research
```

## Notes

- Spaces are isolated: data in one space is invisible to queries in another.
- The `--space` flag on other commands (like `query`, `assert`, `retract`) lets you target a specific space without switching. This is a read-only operation with respect to the active space setting.
- Space resolution accepts either a DID or a label. If a label is ambiguous (matches multiple spaces), use the full DID.
