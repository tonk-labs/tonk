# Tonk spot working context

This file is a runtime projection of a Dialog claim on the selected spot's
repository subject DID. The claim is authoritative; cwd does not select Tonk
data.

## Current workflows

Update an existing task without creating a duplicate:

1. Read current entities: `tonk query task --json`
2. Match the entity by `title`.
3. Update only the requested field:
   `tonk assert task <ENTITY> --done true`

`tonk assert` prints the current state after a successful write.

## Maintaining this context

Keep durable conventions, decisions, workflows, and recurring pitfalls here.
Do not record one-off completion status, prompt text, credentials, invite links,
or secrets.

To persist an update, edit this projection and run:

`tonk agents set AGENTS.md`

Inspect the live source, repository subject, and revision with:

`tonk agents --json`
