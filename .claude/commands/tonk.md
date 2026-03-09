# tonk CLI — Agent Reference
tonk is a CLI for managing spaces (context repositories) with DID-based identity. Spaces store structured data through **concepts** (schemas) and **entities** (entries). All commands support `--json` for structured output.

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

A concept is a named schema that defines what attributes can be associated with entities. Attribute names are auto-prefixed with the lowercase concept name (for CLI-defined concepts) or the YAML-declared namespace (for imported concepts). For example, concept "Task" → attributes stored as `task/title`, `task/status`. Description is required (`--description` flag).

```bash
# List all concepts in the active space
tonk --json concept
# → [{"name":"Task","count":12},{"name":"Contact","count":5,"description":"People and orgs"}]

# Define a new concept (attributes are short names, auto-prefixed with concept name)
tonk --json concept define Task title status priority --description "A task to track"
# → {"ok":true,"name":"Task","namespace":"task","attributes":["task/title","task/status","task/priority"],"description":"A task to track"}

# Interactive mode (prompts for attributes one by one)
tonk concept define Task --description "A task to track"

# Show concept schema
tonk --json concept show Task
# → {"name":"Task","namespace":"task","attributes":["task/title","task/status","task/priority"],"count":12,"entity":"did:key:z..."}

# Add attributes to an existing concept
tonk --json concept extend Task due_date assignee
# → {"ok":true,"added":["task/due_date","task/assignee"]}

# Delete a concept (fails if matches exist unless --force)
tonk --json concept delete Task --force
# → {"ok":true,"deleted":"Task","count_deleted":12}
```

### Convergent define (idempotency & conflict handling)

`concept define` uses convergent semantics. The concept entity is derived deterministically from its attribute set, so defining the same concept twice is a noop:

```bash
# Re-defining with identical attributes converges (noop)
tonk --json concept define Task title status priority --description "A task to track"
# → {"ok":true,"converged":true,"name":"Task","attributes":["task/title","task/status","task/priority"],"description":"A task to track"}
```

If a concept with the same name but **different attributes** already exists, the JSON response returns a conflict with full details instead of an error:

```bash
# Defining with different attributes produces a conflict response
tonk --json concept define Task title status priority due_date --description "Updated task schema"
# → {"ok":false,"conflict":true,"name":"Task",
#    "existing_entity":"did:key:z...",
#    "existing_attributes":["title","status","priority"],
#    "proposed_entity":"did:key:z...",
#    "proposed_attributes":["title","status","priority","due_date"],
#    "message":"A different concept already exists under the name 'Task'. Re-run with --update to replace it (a provenance link will be created), or choose a different name."}
```

When a conflict is detected in JSON mode, the caller should inspect `existing_attributes` vs `proposed_attributes` and decide whether to update or rename. In interactive mode (without `--json`), the user is prompted to choose and can provide a rationale for the update, which is stored as `concept/update-rationale` on the new concept entity. Updated concepts link to their predecessor via `concept/prior` for provenance tracking.

## Creating entities — `tonk create`

```bash
# From key=value pairs (keys auto-prefixed to concept namespace)
tonk --json create Task title="Fix login bug" status=todo priority=high
# → {"ok":true,"id":"did:key:z...","concept":"Task","data":{"title":"Fix login bug","status":"todo","priority":"high"},"created":1739012345}

# From JSON on stdin
echo '{"title":"Fix login bug","status":"todo"}' | tonk --json create Task --stdin

# From a JSON file
tonk --json create Task --file task.json
```

The entity ID (`did:key:z...`) is deterministically derived from the entity's field content and returned in the response. Store it to reference the entity later.

## Querying entities — `tonk query`

```bash
# All entities of a concept
tonk --json query Task
# → [{"id":"did:key:z...","data":{"title":"Fix login bug","status":"todo","priority":"high"}},...]

# Filter by a single attribute (fast — uses value index)
tonk --json query Task status=todo

# Filter by multiple attributes (client-side filtering)
tonk --json query Task status=todo priority=high
```

## Entity operations — `tonk show`, `tonk assert`, `tonk retract`

```bash
# Show full details of an entity
tonk --json show "did:key:z..."
# → {"id":"did:key:z...","concept":"Task","data":{"title":"Fix login bug","status":"todo","priority":"high"},"created":1739012345}

# Assert (update) specific fields (keys auto-prefixed)
tonk --json assert "did:key:z..." status=done
# → {"ok":true,"id":"did:key:z...","updated":[{"status":"done"}]}

# Retract an entity (soft retraction — marks facts as retracted)
tonk --json retract "did:key:z..."
# → {"ok":true,"id":"did:key:z...","concept":"Task"}
```

Note: Entity-to-concept inference is best-effort. An entity may have attributes spanning multiple concepts, so `show` reports the best matching concept based on attribute overlap.

## Batch operations — `tonk batch`

Create, update, or delete multiple entities atomically in a single commit. Input is a JSON array via `--file` or `--stdin`. If any item fails validation, the entire batch aborts with no changes committed.

```bash
# Batch create — array of attribute objects
echo '[{"title":"Fix bug","status":"todo"},{"title":"Write docs","status":"todo"}]' \
  | tonk --json batch create Task --stdin
# → {"ok":true,"concept":"Task","count":2,"created":[{"id":"did:key:z...","data":{...}},...]}}

# From a file
tonk --json batch create Task --file tasks.json

# Batch update — each object must include "id" plus fields to change
echo '[{"id":"did:key:z...","status":"done"},{"id":"did:key:z...","priority":"high"}]' \
  | tonk --json batch update Task --stdin
# → {"ok":true,"concept":"Task","count":2,"updated":[{"id":"did:key:z...","updated":[{"status":"done"}]},...]}}

# Batch delete — array of entity ID strings
echo '["did:key:z...","did:key:z..."]' \
  | tonk --json batch delete Task --stdin
# → {"ok":true,"concept":"Task","count":2,"deleted":["did:key:z...","did:key:z..."]}
```

### Input formats

| Operation | Input shape | Required fields |
|-----------|------------|-----------------|
| `batch create` | `[{...}, ...]` | Attribute key-value objects |
| `batch update` | `[{...}, ...]` | Each object must have `"id"` plus fields to change |
| `batch delete` | `["id", ...]` | Array of entity ID strings |

## Spaces

```bash
tonk --json space                    # List accessible spaces
tonk --json space current            # Show active space
tonk --json space create "name"      # Create a new space
tonk space set "name-or-did"         # Switch active space
```

Concepts and entities are scoped to the active space. Switching spaces gives you a different set of concepts and data.

## Rules — deriving entities from patterns

A rule defines how entities of a concept can be **derived** from patterns across existing facts. Rules use Datalog-style deduction: positive premises (`when`) that must hold, and negative premises (`unless`) that must NOT hold. Variables (prefixed with `?`) bind values and create implicit joins across premises.

Multiple rules can derive the same concept — they act as **OR branches**. Each rule independently produces conclusions that match the conceptual model, and all conclusions are combined forming a unified set. This lets you evolve rules independently rather than maintaining one giant rule with many branches.

When you query a concept that has rules, conclusions from all applicable rules are transparently merged with directly asserted data.

```bash
# List all rules
tonk --json rule
# → [{"entity":"did:key:z...","name":"safe-meals","conclusion":"SafeMeal"},...]

# Define a named rule from JSON (via file)
tonk --json rule define safe-meals --file rule.json --description "Meals safe for all attendees"

# Define an unnamed rule (name is optional; entity ID derived from definition hash)
tonk --json rule define --file rule.json --description "Meals safe for all attendees"

# Define a rule from JSON (via stdin)
cat <<'EOF' | tonk --json rule define safe-meals --stdin --description "Meals safe for all attendees"
{
  "conclusion": {
    "concept": "SafeMeal",
    "bindings": {
      "attendee": "?person",
      "recipe": "?recipe_name"
    }
  },
  "when": [
    { "the": "allergy/person", "of": "?this", "is": "?person" },
    { "the": "recipe/name", "of": "?recipe", "is": "?recipe_name" },
    { "the": "recipe/ingredient", "of": "?recipe", "is": "?ingredient" }
  ],
  "unless": [
    { "the": "allergy/substance", "of": "?this", "is": "?ingredient" }
  ]
}
EOF
# → {"ok":true,"entity":"did:key:z...","name":"safe-meals","conclusion":"SafeMeal","when_count":3,"unless_count":1}

# Show rule details (by name or entity ID)
tonk --json rule show safe-meals
tonk --json rule show did:key:z...

# Delete a rule (by name or entity ID)
tonk --json rule delete safe-meals
tonk --json rule delete did:key:z...

# Query now includes derived entities transparently
tonk --json query SafeMeal
```

### Rule definition JSON format

```json
{
  "conclusion": {
    "concept": "ConceptName",
    "bindings": { "attr_short_name": "?variable", ... }
  },
  "when": [
    { "the": "namespace/attribute", "of": "?var_or_constant", "is": "?var_or_constant" },
    ...
  ],
  "unless": [
    { "the": "namespace/attribute", "of": "?var_or_constant", "is": "?var_or_constant" },
    ...
  ]
}
```

**Conclusion fields:**
- `concept` — the name of the concept being derived (must already be defined; referencing an undefined concept is an error)
- `bindings` — maps concept attribute short names to premise variables (`"?var"`) or constant values (any string without `?` prefix). The variable `?this` is implicit and refers to the derived entity's identity — use it directly in premises (e.g. `"of": "?this"`). The `"this"` key must not appear in bindings. Not every premise variable needs to appear in bindings — variables can appear only in premises to serve as join variables (e.g. `?ingredient` in the example above joins `when` and `unless` premises without appearing in the conclusion). The rule compiler automatically renames variables to match the concept's operand names; if this would cause a collision with another variable, the conflicting variable is auto-renamed.

**Premise terms:**
- `"?name"` — variable (binds/joins by name across premises)
- `"?this"` — implicit entity identity variable (refers to the derived entity)
- `"_"` — wildcard (matches anything, no binding)
- any other string — constant (exact match filter)

**Constraints:**
- `the` is always a fully qualified attribute name (constant). Premise attributes reference existing data but are not validated against concept schemas — only the conclusion concept must be defined.
- `?this` must appear in at least one positive (`when`) premise to ground the entity identity
- Every variable in conclusion `bindings` must appear in at least one positive (`when`) premise (appearing only in `unless` is not sufficient — this is a Datalog safety requirement)
- The conclusion concept must already be defined
- The `unless` section is optional (negation-as-failure)
- Rule names are optional. When provided, names are case-insensitive (stored and looked up in lowercase) and serve as human-friendly identifiers. When omitted, a deterministic entity ID is derived from the definition hash. Unnamed rules can be referenced by entity ID for show/delete.
- Rules must have at least one positive premise in `when`

## Typical workflow

```bash
# 1. Orient
tonk --json status
tonk --json concept

# 2. Define a schema (if needed)
tonk --json concept define Task title status priority --description "A task to track"

# 3. Create entities
tonk --json create Task title="Fix login bug" status=todo priority=high
tonk --json create Task title="Write tests" status=todo priority=medium

# 4. Query and filter
tonk --json query Task status=todo

# 5. Assert updates as work progresses
tonk --json assert "did:key:z..." status=done

# 6. Sync with remote
tonk sync
```

## Full command reference

| Command | Purpose |
|---------|---------|
| `tonk --json status` | Current context (operator, session, space) |
| `tonk --json concept` | List all concepts in the active space |
| `tonk --json concept define <name> [attrs...] --description "..."` | Define a new concept |
| `tonk --json concept show <name>` | Show concept schema |
| `tonk --json concept extend <name> <attrs...>` | Add attributes to a concept |
| `tonk --json concept delete <name> [--force]` | Delete a concept |
| `tonk --json rule` | List all rules in the active space |
| `tonk --json rule define [name] --file <json> --description "..."` | Define a rule from JSON file (name optional) |
| `tonk --json rule define [name] --stdin --description "..."` | Define a rule from stdin JSON (name optional) |
| `tonk --json rule show <name-or-entity-id>` | Show rule definition |
| `tonk --json rule delete <name-or-entity-id>` | Delete a rule |
| `tonk --json create <concept> [key=val...]` | Create an entity |
| `tonk --json query <concept> [key=val...]` | Query/filter entities |
| `tonk --json show <id>` | Show entity details |
| `tonk --json assert <id> [key=val...]` | Assert (update) entity fields |
| `tonk --json retract <id>` | Retract an entity |
| `tonk --json batch create <concept> --file/--stdin` | Batch create entities from JSON array |
| `tonk --json batch update <concept> --file/--stdin` | Batch update entities (objects with "id") |
| `tonk --json batch delete <concept> --file/--stdin` | Batch delete entities (array of IDs) |
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
