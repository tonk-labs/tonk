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
