# tonk CLI — Agent Reference

tonk is a CLI for managing spaces (context repositories) with DID-based identity. Spaces store context that persists across sessions — explorations, findings, decisions, artifacts. All commands support `--json` for structured output.

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

Always run `tonk --json status` first to orient before creating new sessions/spaces.

## Storing context — `tonk remember`

```bash
# Simple note (defaults to topic "general", kind "note")
tonk --json remember "The auth module uses UCAN delegation chains"

# With topic and kind
tonk --json remember --topic "auth" --kind "decision" "We chose Ed25519 over secp256k1"

# From stdin (for large content or piped output)
cat summary.md | tonk --json remember --topic "paper-123" --kind "artifact"

# From file
tonk --json remember --topic "paper-123" --kind "artifact" --file ./output.md
```

Output: `{"ok":true,"id":"did:key:z...","topic":"auth","kind":"decision","timestamp":1739012345,"summary":"..."}`

Items are deduplicated by content+topic hash. Re-remembering the same content updates the timestamp.

**Kinds** are free-form strings. Common conventions: `note`, `decision`, `finding`, `artifact`, `summary`, `error`, `question`.

## Retrieving context — `tonk recall`

```bash
# By topic — everything under "auth"
tonk --json recall "auth"

# By kind — all decisions
tonk --json recall --kind "decision"

# By topic and kind
tonk --json recall "auth" --kind "decision"

# Most recent N items across all topics
tonk --json recall --recent 5

# Specific item by ID (from a previous remember/recall result)
tonk --json recall --id "did:key:z..."
```

Output: `[{"id":"...","topic":"auth","kind":"decision","timestamp":1739012345,"content":"..."},...]`

Large content (>500 chars) is truncated in list results with `"truncated":true`. Use `--id` to get full content.

## Discovering what's stored — `tonk context`

```bash
# Space summary — topics, kinds, item counts
tonk --json context
# → {"space":{"did":"...","name":"..."},"topics":[{"name":"auth","items":12,"latest":1739012345}],"kinds":{"note":25,"decision":8},"total_items":42}

# Drill into a topic — summaries of all items
tonk --json context "auth"
# → {"topic":"auth","items":[{"id":"...","kind":"decision","timestamp":...,"summary":"..."},...]}`
```

**Always start with `tonk --json context`** to see what's in a space before recalling specific items.

## Spaces

```bash
tonk --json space                    # List accessible spaces
tonk --json space current            # Show active space
tonk --json space create "name"      # Create a new space
tonk space set "name-or-did"         # Switch active space
```

## Full command reference

| Command | Purpose |
|---------|---------|
| `tonk --json status` | Current context (operator, session, space) |
| `tonk --json context` | What's stored in the active space |
| `tonk --json context <topic>` | Drill into a topic |
| `tonk --json remember [--topic T] [--kind K] <content>` | Store context |
| `tonk --json recall <topic>` | Retrieve by topic |
| `tonk --json recall --kind <kind>` | Retrieve by kind |
| `tonk --json recall --recent <n>` | Most recent items |
| `tonk --json recall --id <id>` | Get full content of a specific item |
| `tonk login --self` | Non-interactive self-auth |
| `tonk login --delegation <file>` | Import delegation from file/base64 |
| `tonk --json space create <name>` | Create a new space |
| `tonk --json space` | List accessible spaces |
| `tonk space set <name-or-did>` | Switch active space |
| `tonk space delegate --to <did>` | Delegate space access |
| `tonk --json session` | List sessions |
| `tonk sync` | Sync with remote |

## Developer tools

Raw fact operations and inspection are under `tonk dev`:

```bash
tonk dev fact assert --the "ns/pred" --of "~/entity" --is value
tonk dev fact find --the "ns/pred"
tonk dev fact batch                  # JSON lines from stdin
tonk dev inspect delegation <input>
tonk dev operator generate
```
