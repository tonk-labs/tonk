# Evolving command concepts

Status: problem statement + proposal. Not implemented.
Date: 2026-07-17

## The problem

A command concept cannot gain a field.

`core.yaml` and `profile.yaml` are seeded into a branch **once, at creation**,
and never re-seeded. There is no version stamp and no migration path. So every
existing space and profile carries a descriptor frozen at whatever version
created it.

A Rust command concept matches a transient by decoding its facts. Every field on
the struct is required. Add one, and the command can only match a transient
carrying that fact — against an older frozen descriptor it matches **nothing**.
`dispatch` commits the transient and runs no handler. The dialog still closes on
its own, so it looks like it worked.

That failure is silent, it is remote (it breaks other people's existing spaces,
not yours), and it is invisible in local development, where every space is new.

## It has now happened twice

**`CreateSpace.remote`** — documented in
`docs/space-sync-remotes-and-launchpad.md` §3.1. A required `remote` field meant
`CreateSpace{name, remote}` could not match the older name-only descriptor.
Would have broken space creation for every existing user. Caught before merge.

**`Invite.space`** (2026-07-17, this branch) — the FAB moved to dispatching
routeless from the profile branch, where the origin repo is empty, so the
handler needed the target space named on the command. Adding `space` as a
required field broke every frozen `tonk:invite` descriptor. Caught by CI
(`router::tests::it_dispatches_the_invite_command`), not by review — and caught
in the very branch whose purpose was to eliminate silent-success failures.

Both were caught by luck and by tests that already existed. Neither was caught
by the mechanism.

## The current escape hatch, and why it is not good enough

The workaround is to keep the matched concept minimal and read the extra value
opportunistically off the raw facts:

- `remote_from_facts` (`tonk-worker/src/router/repository.rs`)
- `template_from_facts` (same)
- `invite_space_from_facts` (same, added by the fix)

It works, and it is the right call given the current tools. But it has a real
cost: **the schema lies.** `Invite` says a command is `{this, time, marker}`
when a live invite genuinely carries a target space. The true contract lives in
a hand-written fact reader in the handler, where no type sees it and nothing
forces a caller to supply it.

Three fields now sit outside the concepts they belong to. That is a pattern, and
the next person to need a field will find `*_from_facts` and copy it, because it
is the only shape in sight.

## The gap

The notation layer **already has optional fields**: `concept!:` supports a
`maybe:` block, and `profile.yaml` uses it (`tonk:space`'s `name`, with a
comment explaining that a required field there would empty the whole Hub).

`#[derive(Concept)]` (in `dialog-db`'s `dialog-macros`) has no equivalent. It
has no knowledge of `maybe` and no `Option<T>` handling, and — consistent with
that — **no Rust concept anywhere in this repo has an optional field.** Rust
concepts are all-or-nothing.

So the notation can express "this field may be absent" and Rust cannot. The
`*_from_facts` helpers exist to fill that hole by hand.

## Proposal

Teach the concept derive optional fields, mapping to the notation's existing
`maybe:` semantics:

```rust
pub struct Invite {
    pub this: Entity,
    pub time: TimeStamp,
    pub space: Option<Space>,   // None against a frozen older descriptor
    pub marker: Invite,
}
```

- An old transient carries no `space` fact and decodes as `None`.
- A new one decodes as `Some`.
- The field lives where it semantically belongs; the type stops lying.
- No bespoke fact reader.
- **The compiler forces the handler to answer "what if it is absent?"** — which
  is exactly the backward-compatibility question the current design lets you
  forget until CI or a user finds out.

This makes additive schema evolution safe *by construction* rather than by
discipline plus postmortem. Today the invariant is enforced by a doc comment on
`CreateSpace`, a section in another document, and whoever remembers to read
them. That is not enforcement.

## Scope and caveats

- This is a **`dialog-db` change**, not a tonk one. Per house rule, it must be
  described generically — optional concept fields — and must never name tonk,
  invite, or any consumer in dialog-db's code or docs.
- It does **not** retire `remote`/`template`. Those are excluded for a second,
  unrelated reason: a URL cannot round-trip as a `String`-typed concept field
  (see `CreateSpace`'s doc). Optional fields fix the frozen-descriptor problem,
  not the value-encoding one.
- Removing or retyping a field stays unsafe. This only makes *addition* safe.
- Worth pairing with a guard test asserting every command still decodes from its
  oldest shipped shape. `fab_drift.rs` deliberately does not cover this: it
  checks that the FAB's *new* claim carries the attributes the *new* handler
  triggers on, which passed happily while `Invite` was broken. The invariant
  that matters is the *old* caller against the *new* handler, and today only
  `tonk-worker`'s own pre-existing wasm tests encode it.

## Why this is easy to lose

Every symptom points somewhere else. The command "fails" by doing nothing at
all; the UI reports success; local dev never reproduces it; and the only trace
is a `transact` log line with no `command` line after it. The two times it has
been caught, it was caught downstream of the mistake, by tests that happened to
exist. A third time will look exactly the same.
