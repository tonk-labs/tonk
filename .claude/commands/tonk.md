# tonk CLI — Agent Reference

tonk is a CLI for managing spaces (collaboration units) and facts (EAV triples) with DID-based identity and UCAN authorization. All commands support `--json` for structured output.

## Bootstrap (first-time setup)

```bash
# Check current context
tonk --json status
# → {"operator":"did:key:z...","session":null,"space":null}

# If session is null, create one (self-auth, no browser needed):
tonk login --self

# If space is null, create one:
tonk --json space create "workspace-name"
# → {"ok":true,"name":"workspace-name","did":"did:key:z...","owners":["did:key:z..."]}
```

Always run `tonk --json status` first to see what's already set up before creating new sessions/spaces.

## Reading facts

```bash
# Query by attribute
tonk --json fact find --the "namespace/predicate"

# Query by entity
tonk --json fact find --of "~/my-entity"

# Query by both
tonk --json fact find --the "meta/tag" --of "~/my-entity"
```

Output: `[{"type":"assertion","the":"...","of":"did:key:z...","is":"..."},...]`

An empty result returns `[]`.

## Writing facts

```bash
# Single fact
tonk --json fact assert --the "namespace/predicate" --of "~/entity-path" --is some value here

# Retract a fact
tonk --json fact retract --the "namespace/predicate" --of "~/entity-path" --is some value here
```

Output: `{"ok":true,"op":"assert","the":"...","of":"did:key:z...","is":"..."}`

## Batch operations

For multiple writes, use batch mode (one JSON object per line on stdin):

```bash
echo '{"op":"assert","the":"meta/tag","of":"~/doc-1","is":"important"}
{"op":"assert","the":"meta/summary","of":"~/doc-1","is":"A summary of the document"}
{"op":"retract","the":"meta/tag","of":"~/doc-1","is":"draft"}' | tonk --json fact batch
```

Output: `[{"ok":true,"op":"assert",...},...]`

Batch commits all operations in a single transaction.

## Key rules

### Attributes
Format: `namespace/predicate` (must contain a `/`). Examples: `meta/tag`, `finding/summary`, `status/health`, `project/description`.

### Entities (--of)
- `~/path` — Operator-scoped. Signed with operator key then hashed. Same path = same entity for the same operator, different entity for different operators.
- Valid URI (e.g., `did:key:z...`, `https://example.com`) — Used as-is.
- Anything else — blake3 hashed to a `did:key`.

Use `~/` paths for things that belong to you. Use URIs for shared/external references.

### Values (--is)
Auto-detected: integers become numbers, everything else becomes a string. Multi-word values are joined with spaces.

## Listing spaces

```bash
tonk --json space
# → [{"did":"did:key:z...","name":"my-space","active":true,"is_auth_space":false},...]

tonk --json space current
# → {"did":"did:key:z...","name":"my-space"}
```

## Switching spaces

```bash
tonk space set "space-name"
# or
tonk space set "did:key:z..."
```

## Full command reference

| Command | Purpose |
|---------|---------|
| `tonk --json status` | Current context (operator, session, space) |
| `tonk login --self` | Non-interactive self-auth |
| `tonk login --delegation <file>` | Import delegation from file/base64 |
| `tonk --json space create <name>` | Create a new space |
| `tonk --json space` | List accessible spaces |
| `tonk --json space current` | Show active space |
| `tonk space set <name-or-did>` | Switch active space |
| `tonk --json fact assert --the A --of E --is V` | Assert a fact |
| `tonk --json fact retract --the A --of E --is V` | Retract a fact |
| `tonk --json fact find [--the A] [--of E] [--is V]` | Query facts |
| `tonk --json fact batch` | Batch ops from stdin (JSON lines) |
| `tonk --json session` | List sessions |
| `tonk --json session current` | Show active session |
| `tonk space delegate --to <did> [-o file]` | Delegate space access |
| `tonk sync` | Sync with remote |
