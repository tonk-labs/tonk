# Space admins

The requirement: some members are admins, and any admin can remove any
member — including members invited by a different admin, and members
invited before this admin existed.

## The shared-key sketch

One shape considered: mint an **admin principal** at space creation —
a keypair the space delegates full access to — and custody its secret
the way the vault key is custodied: sealed copies, one per admin's
account, minted when an admin is invited. Every member invite is then
issued BY the admin principal, so each admin holds the issuer key of
every member chain and can mint issuer revocations for any of them.

What it gets right: revocation authority that reaches chains minted
before the admin existed, and distribution machinery (conceal/reveal,
sealed copies as facts) that already exists for the vault.

## Why delegated revocation may make the shared key unnecessary

The device registry work landed exactly this mechanism one level up:
revoking a device does not require being the issuer of its grant — any
principal that can PROVE full authority for the subject can mint a
delegated revocation, and the access service verifies the proof path
(dialog #748, tonk #745). An admin holding `space -> admin` (full, for
that space) can prove for the space subject, which is exactly the
authority revoking any chain hanging off the space requires.

So per-admin delegations suffice for the requirement:

- **Kick any member, regardless of who invited them**: the member's
  chain hangs off the space; the admin proves for the space; delegated
  revocation. No shared key.
- **Add an admin later**: the founder's account (or any admin — the
  grant is subject-scoped and re-delegable) mints
  `space -> founder -> new-admin`. The new admin can immediately revoke
  chains minted before they existed, because the authority is over the
  SUBJECT, not over the issuer.

## What the shared key actually costs

- **No attribution.** Every admin is literally the same principal;
  chains cannot say which admin invited or kicked whom.
- **Demotion is broken both ways.** An ex-admin still knows the key —
  revoking their sealed copy stops future secrets, not the one in their
  head. Rotating the admin principal invalidates EVERY member chain
  issued under it (they all hang off the old key), so removing one
  admin means re-inviting every member. With per-admin delegations,
  revoking an admin kills only the chains that admin issued — a
  bounded, attributable blast radius — and the delegated-revocation
  power of the remaining admins is untouched.

## Proposal

- `MemberRole` grows an `admin` value; the role fact lives on the
  membership entity like everything else.
- Admin authority is a per-admin delegation:
  `space -> founder-account -> admin-account`, subject-scoped to the
  space, full commands, retained in the space DB like every membership.
- Kicking is a delegated revocation minted under the kicker's own admin
  chain — the exact code path device revocation uses today, pointed at
  a member chain instead of a device grant.
- The sealed-custody machinery stays reserved for the SPACE key seed
  (`CustodiedSeed`): custody of the seed is what allows re-issuing the
  space's delegations at rotation, and sealing THAT to admin accounts —
  later, if wanted — is how admins could become recovery custodians
  without sharing a signing identity day to day.

Open question: whether kicking should also rotate anything. A kicked
member keeps bytes they already replicated; revocation cuts off
storage, not memory. That is the same honesty note the vault plan
carries, and it is policy, not schema.

## The revocation rule, precisely (2026-08-24)

Two halves, both in code:

1. Dialog asks the revocation checker per link, with the candidate
   revokers for a link being the subject and the issuers of the hops
   above it (`IndexedRevocations::query`, tonk-access-service). A
   revocation recorded under a sibling or a recipient never reaches
   anyone; authority to revoke flows downward only.
2. The service records a revocation under the invocation's `sub`
   (`revoke.rs`). For a delegated revocation that is the subject the
   `prf` chain proves for: the space. The space is the root issuer of
   every chain under it, so a revocation proven for the space applies
   to every hop in the space, whoever minted the hop and whenever.

So the proposal above holds, with one correction to its premise: it is
not "an admin can prove for the subject" that matters, it is that the
proof carries `/ucan/revoke`. Every member currently can: invites are
minted from an unattenuated claim, so every member's chain is `/` and
every member can remove anyone, founder included, through the existing
invite-revocation route.

Admin therefore has to be decided by what a chain covers:

- Member invites are attenuated to the storage commands the presign path
  authorizes, `/memory` and `/archive`. They share no prefix, so a
  member invite is two delegations (one chain per command), retained,
  claimed, and proved side by side. A member cannot invoke
  `/ucan/revoke` for the space at all.
- The founder's `space -> account` prefix and an admin's
  `space -> ... -> admin-account` chain stay `/`. Promotion mints the
  admin chain the way a scoped invite is minted, retains it into the
  space db, and stamps `MemberRole::admin`.
- Removing a member is a delegated revocation of the member's own hop,
  proven for the space under the kicker's `/` chain. For that hop to be
  individually revocable it must be in the space db: the join retains
  the claimed chain into the content branch (the invite hop alone is
  shared by everyone who used an open invite).
- Existing members hold `/` chains until re-invited; they are de facto
  admins and should be listed as such or re-issued.

## Status (2026-08-25)

- dialog-db#469 moves every storage command under `/use` with HTTP
  verbs (`/use/get/memory/cell`, `/use/put/archive/block`, ...), with
  `Use` a level of the capability hierarchy. `/use` is read+write,
  `/ucan/*` stays outside it, `/void` is reserved and empty.
- On `feat/space-admins`: invites (worker route, `tonk:invite` command,
  CLI) are minted at `/use`; invite proofs are searched at `/use`;
  customer deposits go through `Use`; the space still delegates `/` to
  its creator's account. `MemberRole::ADMIN`, the roster badge and the
  CLI classification exist. The join retains the claimed chain into the
  space's content branch so a member's own hop is individually
  revocable.
- `member/promote` and `member/expel` are commands run by worker handlers
  after the commit, like `tonk:invite`. Expel is a form on the space
  branch: it proves the member's own hop, revokes it under the remover's
  `/` chain, records it at the access service and retracts the roster
  rows. Promote is dispatched by the FAB's roster: the guest asks the
  outer page to delegate `/` on the space to the member's account
  (`window.tonk.delegate`), the page runs the passkey ceremony inside
  the click's activation and answers with the root-signed hop, and the
  command carries it. The worker proves the promoter's own `/` chain,
  appends the hop, checks issuer, audience, subject and command, retains
  it beside the invites and stamps the role. No device key sits in an
  admin chain, so signing a device out never takes its promotions with
  it. No HTTP endpoints.
