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

One wrapping per principal that should be able to open the vault:

- **per profile** — the device's own key, so ordinary use needs nothing else
- **per passkey** — so a new device can open the vault at login
- **under the account root** — so rotation can re-wrap for principals the
  rotating device cannot otherwise reach

The root wrapping is not optional. Rotation performed from device A can only
re-wrap for keys A can derive, and A cannot derive passkey B's symmetric key. So
without a root-held copy, any passkey absent at rotation time is locked out
permanently by a rotation it had no part in.

> [!caution]
> The account root is described as an ephemeral genesis keypair, destroyed after
> signing its two delegations — its DID names the account, but nobody holds the
> key. A wrapping literally under the root would therefore be unopenable. It has
> to sit under something durable standing in for the root, and the **recovery
> anchor** is the candidate: a real keypair with visibly delegated authority,
> already mandatory at genesis for the same reason (no direct root delegation can
> be minted later). That makes vault recovery email-OTP-gated, matching passkey
> enrolment.
>
> The alternative — retaining the root key — avoids the service dependency but
> reintroduces a durable stealable secret. That is the trade to decide, not an
> implementation detail.

## Rotation and revocation

These are one operation, not two. Removing a device's wrapping stops it
receiving *future* secrets, but it already knows the current vault key — so
genuine removal means re-keying and re-encrypting everything stored under the
old key.

Rotation needs authority to re-wrap for every remaining principal, so it needs a
passkey (or the anchor path). The natural shape is: revoking a device **offers**
rotation, stated honestly — without it, the revoked device keeps what it already
has; with it, that access dies too, at the cost of a re-wrap per principal.

- **Key id.** Even before rotation is built, stored ciphertext should carry the
  id of the vault key it was encrypted under, so rotation can be added without a
  migration and a partially re-encrypted store stays readable.
- **What moves first.** `tonk-guest-invite-v1:*` is the obvious first tenant —
  it is a bearer secret with no recovery path today — but only once the guest
  chain no longer depends on a rotating operator DID (see the stable-operator
  change), since that removes the need to retain it at all.
