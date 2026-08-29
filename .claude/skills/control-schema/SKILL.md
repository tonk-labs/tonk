---
name: control-schema
description: Keep docs/access-control-schema.md in step with the access service's D1 tables. Use whenever adding a file to rust/tonk-access-service/migrations/, editing a CREATE TABLE or ALTER TABLE there, or changing a struct in rust/tonk-access-service/src/store.rs that mirrors a row.
allowed-tools: Read, Bash, Glob, Grep, Edit, Write
---

# The control schema has a diagram, and it is not generated

`docs/access-control-schema.md` draws the `CONTROL` D1 database as
mermaid: an ER diagram of the tables, a flowchart of the gate that reads
them, and a state diagram of the registration lifecycle.

Nothing generates it. A migration that lands without the diagram moving
leaves a document that is confidently wrong, which is worse than none —
the next reader trusts it and builds on a column that is not there.

## When this applies

- a new file in `rust/tonk-access-service/migrations/`
- a `CREATE TABLE`, `ALTER TABLE`, `ADD COLUMN`, `DROP COLUMN`, or
  `RENAME COLUMN` in an existing one
- a changed field on `Customer`, `Consumer`, or a sibling row struct in
  `rust/tonk-access-service/src/store.rs`
- a change to `provisioning::screen`, which the flowchart draws
- a new `CustomerStatus` variant, which the state diagram draws

## What to update

The tables in the document are the state **after every migration**, not
the contents of any one file. So a migration adding a column edits the
entity block; a migration renaming one edits it in place rather than
appending a note.

1. **The ER diagram** — the column, its type, and a comment when the
   name does not carry the meaning (`expires`, `limit_resets`, and
   `verified` all need one; `email` does not).

   Every column is `TEXT` or `INTEGER`, which says nothing about what a
   value *is*. So say it: a column holding a DID reads
   `"DID (did:key): ..."`, and one that merely looks like it might —
   `plan`, `stripe_customer` — says what it is instead and that it is
   **not** a DID. Same for an enum: spell the variants after `enum:`
   rather than leaving `TEXT`.
2. **Relationships**, when a foreign key appears or moves. The crow's
   feet say the cardinality: `||--o{` is one-to-many, `||--||` one-to-one.
3. **The flowchart**, when the gate reads something new or refuses for a
   new reason. Every leaf should be reachable by reading `screen`.
4. **The lifecycle**, when a status is added or a transition changes.
5. **Prose**, when a change makes a paragraph false. The "not yet built"
   list shrinks as tables arrive.

## Getting it right

Read the migrations rather than the structs. A struct can lag the
schema, carry a field no column backs, or omit one nothing reads yet —
the SQL is what the database has:

```
cat rust/tonk-access-service/migrations/*.sql
```

Two conventions the document keeps:

- **Draw what exists.** `account` appears in the diagram as a
  relationship but is not a table, and the document says so directly
  underneath. Do that rather than drawing a table that isn't there.
- **Say why a constraint exists**, where the reason is not obvious from
  the name. Custody reservations expire because a PRF-derived DID is
  re-derivable, so holding one forever would strand the account. A
  reader who does not know that will remove the expiry.

## The test that checks this

`rust/tonk-access-service/tests/schema_doc.rs` applies every migration to
an in-memory database and compares the result against the `erDiagram`
block, table by table and column by column. A missing column and an
invented one both fail, each naming what is wrong.

```
cargo test -p tonk-access-service --features integration-tests --test schema_doc
```

It exists because this document drifted while a skill said not to let it:
the diagram carried a `ledger` column no migration ever created, and
omitted the `access` one that does exist. Prose asking for diligence did
not survive a rename; a red test does.

The test reads structure only. Comments, relationships, the flowchart and
the lifecycle are still yours to keep true.

## Checking your work

Mermaid fails silently when a block is malformed — the diagram renders
as an error box rather than a wrong picture, so it is worth a look
rather than a compile. Both GitHub and an artifact render these natively.

There is no test for this. The migrations are the source of truth and
the document is prose about them, so the only check is reading both.
