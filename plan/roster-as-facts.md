# Rosters as facts

Two roster-shaped stores predate the account DB and should converge on
it: the profile roster behind the account switcher, and the projection
machinery that fans the account display name out to space rosters.

## The profile roster

Today: a JSON-serialized `Vec<RosterEntry>` in one credential site on
the registry profile (`tonk-profile-roster-v1`), plus an active-profile
pointer site. Every update is a read-modify-write of the whole blob, so
concurrent worker writes race, and entries are stale by design — a
rename on another device shows up only on the next activation here.

Instead: one fact per profile in the registry profile's own DB, and the
active pointer as a fact. Fact merge removes the lost-update race, and
the switcher becomes a subscription instead of a load-once render.

Each entry keeps only what is not derivable elsewhere: the profile's
storage name and a cached display label, refreshed as per-entity asserts
at the moments the worker already has the facts in hand (boot, link,
rename, switch). **No `last_active_at`**: it churns the persisted DB on
every switch and the switcher does not need it.

Identity fields keep one home in the account space: display name is
`AccountDisplayName`, email becomes an account-space fact when the
summary proxy retires. The roster caches a label per closed profile and
nothing more.

## Space membership

Membership is **by account**, and it stays on the space's content
branch — other members must read it without your account DB, so the
per-space copy is irreducible.

Near term, the plain records stay: `Membership` (entity derived from
`(subject, member)`), `MemberRole`, `MemberName`, asserted at the moment
the authority is created and keyed to the account, so a device-keyed row
is never born post-link.

The rename fan-out moves from the account sweep to the rename command's
own post-commit effect: assert `AccountDisplayName`, then enqueue the
per-space `MemberName` updates through the sync queue. The sweep keeps
only the idempotent catch-up, and most of the convergence-report and
retry bookkeeping goes.

Whether a rename propagates everywhere or a member keeps per-space names
is policy, not schema: `MemberName` lives on the membership entity, so a
per-space override is already representable. Default to propagation,
never overwrite a name a member set for that space.

## Sketch: membership as a delegation

The device registry retirement gives the shape: a device IS its
`account -> profile` powerline, retained into the account DB, described
by facts on the delegation's own entity, listed by audience. Membership
can be the same pattern one level down: the claimed invite chain,
retained into the SPACE's DB, is the membership — signed, provable, and
already what authorization walks. `MemberName`/`MemberRole` are then the
description facts on that entity, which is less invasive than a metadata
field inside the UCAN itself.

Joining is the moment the chain is retained, so "who is a member" is
"which accounts do retained chains reach" — the same audience question
the device list answers. A verifiable join (an invocation citing the
invite) or a verifiable name assertion can layer on later without
changing the storage shape.

Joining requires a **registered** account. The onboarding account a
device mints at first boot exists so the device can create local spaces
before sign-up, not so it can enter shared ones: a membership held by an
account whose custody dies with one device is a roster entry nothing can
recover or hold to account. The invite claim is the enforcement point.

One idea deliberately parked: membership as a delegation to read the
member's profile/account DB (a personal card). The account DB holds
sealed secrets, and branches of one repository share blocks, so sharing
any branch of it with a space is sharing block reachability with that
space. If a readable card is wanted, it should be its own small
repository, not a branch of the account's.
