# Evolving command concepts

Status: the mechanism already exists and is unused. Nothing to build; something
to adopt.
Date: 2026-07-17

## The trap

A command concept cannot gain a **required** field.

`core.yaml` and `profile.yaml` are seeded into a branch **once, at creation**,
and never re-seeded. There is no version stamp and no migration path, so every
existing space and profile carries a descriptor frozen at whatever version
created it.

A Rust command concept matches a transient by decoding its facts. Add a required
field and the command can only match a transient carrying that fact — against an
older frozen descriptor it matches **nothing**. `dispatch` commits the transient
and runs no handler. The dialog still closes on its own, so it looks like it
worked.

The failure is silent, remote (it breaks other people's existing spaces, not
yours), and invisible in local development, where every space is new. The only
trace is a `transact` log line with no `command` line after it.

## It has happened twice

**`CreateSpace.remote`** — a required `remote` field could not match the older
name-only descriptor. It would have broken space creation for every existing
user, and was caught before merge.

**`Invite.space`** (2026-07-17) — the FAB moved to dispatching routeless from
the profile branch, where the origin repo is empty, so the handler needed the
target space named on the command. Added as a required field; broke every frozen
`tonk:invite` descriptor. Caught by CI
(`router::tests::it_dispatches_the_invite_command`), in the very branch whose
purpose was to eliminate silent-success failures.

## The mechanism already exists

`#[derive(Concept)]` **supports optional fields today**, on the currently pinned
dialog-db (`tag = tonk-2026-07-14-3`). It is not a `maybe` attribute and not
proc-macro syntax detection — it is pure trait dispatch on the `Option<N>` shape:

- `dialog-query/src/concept.rs` — `impl<N> ConceptField for Option<N>` with
  `const OPTIONAL: bool = true`. Its `term()` builds an optional-typed term;
  `AttributeQuery` reads `is.is_optional()` and switches to the **Absent-fallback
  evaluation path**.
- `dialog-macros/src/query/concept.rs` — the derive documents both paths and
  asserts the one real constraint:
  > "a Concept must declare at least one required (non-Option) attribute field;
  > a concept built only from optional fields constrains nothing and matches
  > every entity"

So the correct fix for `Invite` was one word:

```rust
pub struct Invite {
    pub this: Entity,
    pub time: TimeStamp,
    pub space: Option<Space>,   // None against a frozen descriptor, Some from the FAB
    pub marker: Invite,
}
```

`time` and `marker` stay required, satisfying the at-least-one rule. An old
transient decodes with `space: None`; a new one with `Some`. The field lives
where it semantically belongs, the type stops lying, no bespoke fact reader is
needed — and the compiler **forces** the handler to answer "what if it is
absent?", which is exactly the backward-compatibility question the current
design lets you forget until CI or a user finds out.

## What we shipped instead, and why

The fix on this branch keeps `Invite` minimal (`{this, time, marker}`) and reads
the space opportunistically off the raw facts via `invite_space_from_facts`,
mirroring `remote_from_facts` / `template_from_facts` in
`tonk-worker/src/router/repository.rs`.

That is correct and backward compatible, but it is the workaround, not the
mechanism. It leaves the schema lying: `Invite` claims a command is
`{this, time, marker}` when a live invite genuinely carries a target space, and
the true contract lives in a hand-written reader where no type sees it and
nothing forces a caller to supply it.

**`Invite.space` should be migrated to `Option<Space>` and
`invite_space_from_facts` deleted.** Small, strictly better, no dependency
change.

`remote` and `template` are a separate case: they are excluded for a second,
unrelated reason (a URL cannot round-trip as a `String`-typed concept field —
see `CreateSpace`'s doc), so optional fields do not retire them.

## The actual gap: discoverability

The mechanism is not missing. It is **undiscovered**. No Rust concept in this
repo has ever had an optional field, so nothing in sight demonstrates the
pattern, and everything in sight demonstrates the workaround (three
`*_from_facts` helpers). The next person to need a field will copy the
workaround, because that is the only shape visible.

The codebase's own guidance stops one step short. `CreateSpace`'s doc explains
the trap precisely and correctly — and never mentions that `Option<T>` is the
way out. §3.1 does the same. Both describe the wall; neither points at the door.

Worth doing:

1. Migrate `Invite.space` to `Option<Space>` as the first worked example.
2. Add the one-line pointer to `CreateSpace`'s doc and to §3.1: *a new field on
   an existing command must be `Option<T>`.*
3. Consider a guard test asserting every command still decodes from its oldest
   shipped shape. Note `fab_drift.rs` deliberately does **not** cover this: it
   checks that the FAB's *new* claim carries the attributes the *new* handler
   triggers on, which passed happily while `Invite` was broken. The invariant
   that matters is the *old* caller against the *new* handler, and today only
   `tonk-worker`'s pre-existing wasm tests encode it.

## How this was missed

Recording it because the search failure is instructive, and repeatable.

Grepping the derive for `maybe` returned nothing — optionality is expressed as
`Option<N>` via trait dispatch, deliberately with no `maybe` surface syntax
(aliased imports like `use Option as Maybe` resolve identically at the type
level, which is *why* the derive avoids syntactic detection). And the derive
lives in `dialog-macros/src/query/concept.rs`, one directory below a
non-recursive glob of `src/*.rs`.

Wrong search term plus wrong search path produced a confident conclusion that
the feature did not exist — and a proposal to build what was already there.
