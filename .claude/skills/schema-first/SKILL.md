---
name: schema-first
description: The YAML library under rust/tonk-core/assets/library/ IS the schema. Every durable thing we store or render needs a documented concept declared there, and the Rust `derive(Concept)` structs follow it. Use whenever adding a `derive(Concept)` struct, an `Attribute` in domain.rs, or a new kind of fact — and whenever asking "where is X stored?"
allowed-tools: Read, Bash, Glob, Grep
---

# The schema is the YAML, not the Rust

`rust/tonk-core/assets/library/*.yaml` is the schema of record. A Rust
`derive(Concept)` struct is an *implementation* of a declaration that
belongs in a YAML document, not a declaration in its own right.

This has drifted badly. Whole domains exist in Rust with no YAML at all —
`xyz.tonk.account`, `xyz.tonk.device`, `xyz.tonk.custody`, `xyz.tonk.roster`,
`invitation`, `remote`, `branch`, `email-status` — and several declared
concepts omit fields the Rust actually writes (`tonk:replica` has no
`profile` or `status`; `tonk:member` has no `subject` or `invitation`).
The result is a schema that looks authoritative and silently is not.

## Why the YAML and not the Rust

- A Rust concept has **no URI**. Its identity is structural — the set of
  `xyz.tonk.*` attributes it writes (`dialog-macros/src/query/concept.rs:510`).
  Nothing about it says what it *is*. The YAML `this:` URI is the name.
- The YAML carries the documentation. `description:` on the concept and on
  every field is the only place the meaning of a fact is written down for
  someone who does not already know it.
- Views, commands and models resolve through YAML concepts. A fact with no
  declaration can never be rendered declaratively, only by hand-written Rust.
- One document to read beats grepping 66 structs across 30 files to find
  what exists.

## The rule

**Anything durable we store or render gets a documented concept in the YAML
library.** Declare it there first; make the Rust match.

This holds even when nothing subscribes through the concept. The FAB
deliberately subscribes to raw `xyz.tonk.*` attributes rather than naming
concepts (see `tonk-fab/src/member_roster.rs:12-15` and four siblings) —
naming one would require per-space seeding and break against older seeds.
That is a fine reason for the FAB to read attributes. It is not a reason to
leave the concept undeclared: the declaration is the documentation and the
schema, independent of who queries through it.

## Which document

- `core.yaml` — seeded onto every space's content branch. Space-scoped
  things: members, boards, sheets, views, invites.
- `profile.yaml` — seeded onto the profile meta branch. Hub-scoped things.
  Its header still warns that seeding the full library cost ~2s on first
  load; that is no longer true and is not a reason to leave a concept
  undeclared.
- Account-scoped facts (device links, custody, customer registration) live
  on the account branch and belong in a document seeded there.

Put a concept in the document whose branch actually holds the facts.

## Field types

`text`, `entity`, `unsigned-integer`, `float`, `boolean`, `bytes`
(`tonk-analyzer/src/analyzer/declaration.rs:881`). `bytes` is what a sealed
envelope declares — see `xyz.tonk.custody/sealed`.

Use `with:` for required fields and `maybe:` for optional ones. A required
field that is sometimes absent makes the whole row unresolvable — that is
the `project_sheet_missing_field_empty` failure.

## Never store an encoded copy of authority

A delegation is stored by retaining the chain:

```rust
branch.delegations().retain(UcanDelegation(chain)).perform(env).await
```

Dialog decomposes issuer, audience, subject, command and expiration onto the
delegation's own entity, so it is queryable. A hex or base58 copy in a
concept field is a second source of truth the prover cannot use.

The exception is an invite: `tonk_schema::command::Authorization` holds a
base58 chain because an invite is a delegation to *someone else* whose
rendered form is a URL — `proof` is the `?access=` parameter. Encoded is the
point there.

Note that a delegation's signed `meta` is **not** decomposed into facts; it
rides inside the envelope and cannot be queried. That is why
`xyz.tonk.device/reason` duplicates it as a fact.

## Checklist before adding a `derive(Concept)` struct

1. Does a YAML concept declare this? If not, write it first, with a
   `description:` on the concept and on every field.
2. Does the declaration list every attribute the Rust writes?
3. Is each field `with:` (required) or `maybe:` (optional) correctly?
4. Is this authority that should be retained as a delegation instead?
5. Does `cargo test -p tonk-worker --test standard_library` still pass?
