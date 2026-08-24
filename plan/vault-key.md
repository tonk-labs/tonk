# Vault key: one shared secret, many wrappings

A single **vault key** shared across every device on an account. Secrets and
credentials are stored in the database encrypted with it, rather than in a
device-local keystore.

The vault key itself is never stored in the clear. It is stored once per
*wrapping*: encrypted under a symmetric key derived from each principal that
should be able to open it (a profile, a passkey). Adding a device or a passkey
is re-wrapping the same key, never re-keying the secrets.

## Flows

### Generate

```
generate vault key
derive symmetric key from profile key
encrypt vault key with profile key
```

### Passkey setup

```
derive symmetric key from passkey
decrypt vault key with profile key
encrypt vault key with passkey
```

### Passkey login

```
derive symmetric key from passkey
derive keypair from passkey
obtain secret from custody stored for passkey
decrypt secret
derive account keypair from secret
replicate account db
generate profile keypair
delegate from account to profile
decrypt vault key                      # with the passkey-derived symmetric key above
derive symmetric key from profile key
encrypt vault key with profile key
save encrypted vault key in the account
```

The `decrypt vault key` step uses the passkey-derived symmetric key from the top
of the flow. Worth stating explicitly: by that point in the sequence the profile
key is the one in focus, and it has no wrapping yet — the passkey wrapping is
what bootstraps the profile wrapping.

## Why

Credentials today live in a device-local credential store, one blob per site
name. That store is not replicated and not recoverable: clearing the browser
profile destroys it. The handoff notes claim the account design "closes a trap
where a local passkey backed strictly local spaces, which would get destroyed if
the browser profile was cleared" — but a guest's retained invite is exactly that
trap, because it lives in the credential store and has no other copy.

With a vault, an encrypted secret is an ordinary row in the account DB: it
replicates to every linked device and survives a wipe, while remaining opaque to
anything that cannot open the vault.

## Wrappings

A wrapping is a sealed copy addressed to a DID — dialog's `conceal`/`reveal`
(dialog#463) seals to the X25519 key derived from a `did:key`'s Ed25519 key, so
a copy can be minted for a principal that has published nothing and has never
been online. Sealed copies are facts on the account branch (see
`plan/profile-db.md`: ciphertext is data, and replicating is the point).

Two wrapping kinds cover every flow:

- **per profile** — sealed to the profile's DID, so ordinary use needs nothing
  but the device's own signer
- **under the account** — sealed to the account DID, revealed by the account
  secret's signer; this is the bootstrap and rotation copy

There is deliberately **no per-passkey wrapping**. A passkey never opens the
vault directly — it opens the ACCOUNT: under the custody-envelope model the
passkey's PRF output derives the KEK that unwraps the account secret, so by the
time any passkey login has happened the device holds the account's signer, and
that signer reveals the account-sealed copy. Login then conceals a fresh copy
for the new profile's DID — the same dance the flows above already perform. A
passkey wrapping would be a second door into a room the login is already
standing in.

Dropping it also dissolves the lockout this section used to caution about.
Wrappings addressed to DIDs are re-minted from PUBLIC keys, all enumerable from
the account space, so rotation performed from any one device can include every
principal — present or not. Nothing needs to be online at rotation time to stay
included.

> [!note]
> The trade this makes is coupling vault access to account custody: a passkey
> alone, without the account-sealed copy, opens nothing. That copy is a fact on
> the branch a device must pull to be a device at all — if you can join, the
> copy came with you. And account-compromise-implies-vault-compromise was
> already true: the account KEK wraps the space and invite seeds.

## Rotation and revocation

These are one operation, not two. Removing a device's wrapping stops it
receiving *future* secrets, but it already knows the current vault key — so
genuine removal means re-keying and re-encrypting everything stored under the
old key.

Rotation re-seals the new vault key to the account DID and each remaining
profile DID — public keys, so any device holding the account may perform it.
The natural shape is: revoking a device **offers** rotation, stated honestly —
without it, the revoked device keeps what it already has; with it, that access
dies too, at the cost of a re-seal per principal and a re-encrypt per secret.

- **Key id.** Even before rotation is built, stored ciphertext should carry the
  id of the vault key it was encrypted under, so rotation can be added without a
  migration and a partially re-encrypted store stays readable.
- **What moves first.** `tonk-guest-invite-v1:*` is the obvious first tenant —
  it is a bearer secret with no recovery path today — but only once the guest
  chain no longer depends on a rotating operator DID (see the stable-operator
  change), since that removes the need to retain it at all.
