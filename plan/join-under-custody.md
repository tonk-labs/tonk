# Join under custody: no guests, no space private keys on disk

Decided 2026-08-25. Supersedes `guest-session-renewal.md` and the guest
half of `bug-57-59.md`.

## What changes

1. **Every join is durable, to whatever account the device has.** An open
   invite is claimed `invite-principal -> account` where the account is
   the passkey root when one is linked and the onboarding account
   otherwise. The invite principal's seed is sealed to that account
   (`CustodiedSeed`, `kind = tonk:invite`) exactly as a space seed is.
   Guest visits, the retained invite URL, the guest lease, its renewal on
   operator rotation, the promote-on-click, and `PendingIntent::DurableJoin`
   with its account-required round trip all go.

2. **Accreditation is a rotation.** When a passkey account is created on
   a device that has an onboarding account, the worker opens every seed
   sealed to the onboarding account (it holds that secret locally, no
   passkey needed for the opening), re-issues `space -> new root` and
   `invite-principal -> new root` from those seeds, re-seals every seed to
   the new account's recipient, retracts the old rows, revokes the
   onboarding account's grant to the device, and destroys the onboarding
   custodian and envelope. `adopt_profile_spaces` (re-issue from the local
   signer) is replaced by this; nothing depends on the creating device
   any more.

3. **Spaces are created with a public key.** The signer exists only long
   enough to mint `space -> account`; the repository credential stored is
   the verifier. Every later act on the space proves through
   `space -> account -> device -> operator`, the way a joined replica
   already does. The only copy of the space secret is the `CustodiedSeed`
   sealed to the account. A device that needs the signer again (rotation,
   a recovery custodian later) opens the seed.

## Why

- A guest was an accountless join. There are no accountless devices any
  more (#745: an onboarding account from first boot), so the guest path is
  a second join with weaker authority and its own storage, renewal, and
  UI, kept alive only by the pre-#745 assumption.
- `DurableJoin` replayed the invite URL after sign-up. With the join
  already durable under the onboarding account and the principal seed
  custodied, accreditation re-roots it; the URL is never needed again and
  can be revoked or expire without losing the membership.
- A space private key in the device's credential store is a copy the
  account cannot revoke and a reason accreditation needs the creating
  device. The sealed seed is the only copy that has to exist; the chain
  does the rest.

## Onboarding account custody

The onboarding account has no remote and no account repository; its
facts live on profile `main`, which is exactly where they end up once the
account becomes profile main's upstream at accreditation. So
`AccountEncryptionKey` and `CustodiedSeed` are written to profile `main`
for both kinds of account; the only difference is who can open them.

`custody_seed` therefore no longer gates on `require_ready_account_state`.
It resolves the recipient as: the published `AccountEncryptionKey` for
the linked root if there is one, else the onboarding account's own
recipient (derived, and published on first use). A device that has
neither is a bug, not a state.

## Stages

Each is one PR, in order. Every stage leaves the tree working.

### Stage 1: durable join to the current account

- `join_invite`: `Durable` no longer requires a linked account. The
  member is `identity::root_did` if present, else `onboarding::did`
  (minting the onboarding account and its device grant if absent, as
  `create_repository` does). `member_did` in `router/account.rs` becomes
  this and stops returning the profile DID.
- `custody_seed`: recipient resolution as above; write to profile `main`.
  The commit arm already seals the invite seed.
- The `tonk:join` command always runs `Durable`. `/api/profile/visit`,
  `/api/repository/{repo}/membership`, `join_guest`, `guest_*`,
  `is_guest_replica`, `mint_guest_grant`, the guest record, and their
  tests are deleted. `sync.rs` loses guest renewal. `session.rs` loses
  the guest branch of chain selection.
- `PendingIntent::DurableJoin`, `notify_account_required`, the
  `AccountRequired` join rejection and its `account_gate.rs` replay in
  tonk-ui, and the FAB's `join spot` button, `fab-join-first` dialog,
  membership check, and `SHARE_UNAVAILABLE` stamp for guests are deleted.
  Share is available to any member: the worker mints from the member's
  chain.
- `route_table.rs` loses the two routes.

### Stage 2: public-key spaces

- `create_repository`: mint `space -> account` with the in-memory
  signer, then `create().with_credential(Credential::from(verifier))`.
  `Repository<SignerCredential>` becomes `Repository<Credential>` on the
  create path. `custody_seed` is no longer best-effort here: a create
  whose seed cannot be sealed is refused, because the seed would
  otherwise be lost.
- Callers that reached for the space signer (`try_access` in
  `adopt_profile_spaces`) switch to opening the seed (Stage 3) or go away
  with it.
- CLI `space new` / `tonk-cli/src/site.rs` create the same way once CLI
  spaces delegate to the account (`cli-space-parity.md`); until then the
  CLI keeps its local signer, unchanged.

### Stage 3: accreditation as rotation

- `account::attach` (after `ensure_account_state`): `rotate_from_onboarding`
  replaces `adopt_profile_spaces`. For every `CustodiedSeed` sealed to the
  onboarding recipient: open with `onboarding::account(state)
  .encryption_key()`, re-issue `subject -> root` (space or invite
  principal, full command), save the chain to the access branch, retain it
  into the account, persist the prefix, seal the seed to the new recipient,
  assert the new row, retract the old.
- Then revoke the onboarding account's grant to the device, demote the
  onboarding custodian (overwrite with its public half) and delete the
  envelope, per `onboarding-accreditation.md` steps 6 and 8. Resumable:
  each seed is independent, and a seed still sealed only to the old
  recipient is simply picked up on the next attempt.
- A passkey account rotating to a new secret is the same function with a
  ceremony-supplied `EncryptionKey` instead of the onboarding one
  (authority-facts step 4). Not in this stage, but nothing here blocks it.

## Not changing

- The access service, invite URL format, and `Invite::claim`.
- Targeted (audience-scoped) invites: no seed, chain already at the
  account.
- The CLI's own join, which already claims durably to its device's
  account.
