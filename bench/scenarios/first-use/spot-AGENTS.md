# Tonk spot: bench

This file is trusted local context for this spot. The `TONK_SPOT` environment
variable selects `bench`; changing directory does not select different Tonk
data.

## Current workflows

Update an existing launch task without creating a duplicate:

1. Read the current entities: `tonk query task --json`
2. Match the entity by `title`.
3. Update only the intended field:
   `tonk assert task <ENTITY> --done true`

`tonk assert` prints the entity's current state after the write. Use
`tonk query task <ENTITY> --json` only when separate verification is useful.

Create a new task only when that is the request:

`tonk assert task --title "example" --done false`

## Durable spot context

- `task.title` is required text.
- `task.done` is a required boolean.
- Existing tasks are updated by entity ID.

Keep this section short. Add durable concepts, decisions, and recurring
pitfalls you discover while working in this spot. Do not record one-off task
completion, transient status, credentials, invite links, or other secrets.
