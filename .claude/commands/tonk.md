# tonk CLI — Agent Reference
tonk is a CLI for managing spaces (context repositories) with DID-based identity. Spaces store structured data through **concepts** (schemas) and **instances** (entries). All commands support `--json` for structured output.

## Bootstrap (first-time setup)
 
```bash
# Check current context
tonk --json status
# → {"operator":"did:key:z...","session":null,"space":null}

# If session is null, create one (opens browser for authentication):
tonk login

# If space is null, create one:
tonk --json space create "workspace-name"
# → {"ok":true,"name":"workspace-name","did":"did:key:z...","owners":["did:key:z..."]}
```

Always run `tonk --json status` first to orient before creating new sessions/spaces.

## Concepts — defining schemas

A concept is a named schema that defines what attributes instances can have. Attribute names are auto-prefixed with the concept's lowercase namespace (e.g. concept "Task" → attributes stored as `task/title`, `task/status`).

```bash
# List all concepts in the active space
tonk --json concept
# → [{"name":"Task","instances":12},{"name":"Contact","instances":5,"description":"People and orgs"}]

# Define a new concept (attributes are short names, auto-prefixed)
tonk --json concept define Task title status priority
# → {"ok":true,"name":"Task","attributes":["task/title","task/status","task/priority"]}

# With a description
tonk --json concept define Contact name email role --description "People and organizations"

# Interactive mode (prompts for attributes one by one)
tonk concept define Task

# Show concept schema
tonk --json concept show Task
# → {"name":"Task","attributes":["task/title","task/status","task/priority"],"instance_count":12,"entity":"did:key:z..."}

# Add attributes to an existing concept
tonk --json concept extend Task due_date assignee
# → {"ok":true,"added":["task/due_date","task/assignee"]}

# Delete a concept (fails if it has instances unless --force)
tonk --json concept delete Task --force
# → {"ok":true,"deleted":"Task","instances_deleted":12}
```

## Creating instances — `tonk create`

```bash
# From key=value pairs (keys auto-prefixed to concept namespace)
tonk --json create Task title="Fix login bug" status=todo priority=high
# → {"ok":true,"id":"did:key:z...","concept":"Task","data":{"title":"Fix login bug","status":"todo","priority":"high"},"created":1739012345}

# From JSON on stdin
echo '{"title":"Fix login bug","status":"todo"}' | tonk --json create Task --stdin

# From a JSON file
tonk --json create Task --file task.json
```

The instance ID (`did:key:z...`) is randomly generated and returned in the response. Store it to reference the instance later.

## Querying instances — `tonk query`

```bash
# All instances of a concept
tonk --json query Task
# → [{"id":"did:key:z...","data":{"title":"Fix login bug","status":"todo","priority":"high"}},...]

# Filter by a single attribute (fast — uses value index)
tonk --json query Task status=todo

# Filter by multiple attributes (client-side filtering)
tonk --json query Task status=todo priority=high
```

## Instance operations — `tonk show`, `tonk update`, `tonk delete`

```bash
# Show full details of an instance
tonk --json show "did:key:z..."
# → {"id":"did:key:z...","concept":"Task","data":{"title":"Fix login bug","status":"todo","priority":"high"},"created":1739012345}

# Update specific fields (keys auto-prefixed)
tonk --json update "did:key:z..." status=done
# → {"ok":true,"id":"did:key:z...","updated":[{"status":"done"}]}

# Delete an instance
tonk --json delete "did:key:z..."
# → {"ok":true,"id":"did:key:z...","concept":"Task"}
```

## Spaces

```bash
tonk --json space                    # List accessible spaces
tonk --json space current            # Show active space
tonk --json space create "name"      # Create a new space
tonk space set "name-or-did"         # Switch active space
```

Concepts and instances are scoped to the active space. Switching spaces gives you a different set of concepts and data.

## Typical workflow

```bash
# 1. Orient
tonk --json status
tonk --json concept

# 2. Define a schema (if needed)
tonk --json concept define Task title status priority

# 3. Create instances
tonk --json create Task title="Fix login bug" status=todo priority=high
tonk --json create Task title="Write tests" status=todo priority=medium

# 4. Query and filter
tonk --json query Task status=todo

# 5. Update as work progresses
tonk --json update "did:key:z..." status=done

# 6. Sync with remote
tonk sync
```

## Full command reference

| Command | Purpose |
|---------|---------|
| `tonk --json status` | Current context (operator, session, space) |
| `tonk --json concept` | List all concepts in the active space |
| `tonk --json concept define <name> [attrs...]` | Define a new concept |
| `tonk --json concept show <name>` | Show concept schema |
| `tonk --json concept extend <name> <attrs...>` | Add attributes to a concept |
| `tonk --json concept delete <name> [--force]` | Delete a concept |
| `tonk --json create <concept> [key=val...]` | Create an instance |
| `tonk --json query <concept> [key=val...]` | Query/filter instances |
| `tonk --json show <id>` | Show instance details |
| `tonk --json update <id> [key=val...]` | Update instance fields |
| `tonk --json delete <id>` | Delete an instance |
| `tonk login` | Authenticate via browser |
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
