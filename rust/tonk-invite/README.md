# tonk-invite

Tonk invite URL format and claim orchestration, shared by CLI and web clients.

This crate defines how a tonk invite is encoded as a URL and how a redeemer
turns that URL into a delegation chain terminating at their own DID. It builds
on `dialog-{credentials, ucan-core, varsig}` only and compiles for both native
and WASM targets, so invites minted by the `carry` CLI are redeemable by the
`tonk-ui` web client and vice versa.

The core type is [`Invite`]: a [`DelegationChain`] plus an [`InviteAudience`]
mode and an optional sync remote. It serializes to a URL today via
[`Invite::to_url`]; QR codes and invite files can slot in as additional
methods without touching the type.

## URL format

```text
<base>?access=<base58-ucan-chain>[&remote=<access-service-url>][#<base58-seed>]
```

- `access`: base58-encoded [`DelegationChain`] bytes. The chain's subject must
  be specific (`Subject::Specific`); chains with `Subject::Any` are rejected,
  because an invite always scopes to a particular repository.
- `remote` (optional): UCAN access service endpoint used for sync.
- `#fragment` (optional): base58 of a 32-byte Ed25519 seed.

The fragment is the *audience* axis. Its presence marks the invite as
audience-open ([`InviteAudience::Open`]): any redeemer can claim it by
redelegating from the embedded ephemeral key. Its absence marks the invite as
audience-scoped ([`InviteAudience::Scoped`]): only the chain's recorded
audience DID can claim. The *subject* axis (which repo) is always scoped and is
orthogonal to this.

[`DEFAULT_BASE_URL`] (`https://hub.tonk.xyz/join`) is the canonical base for
minted links.

## Mint and claim

[`Invite::new`] assembles an invite from its parts. It rejects a non-specific
subject, and for an open invite it verifies the embedded seed derives the
chain's tail audience DID, turning a would-be claim-time failure into an
up-front error. [`Invite::parse_url`] decodes a URL back into an `Invite`,
applying the same validation (plus a strict 32-byte length check on the
fragment seed).

[`Invite::claim`] redeems an invite for a redeemer's `Did` and returns a
[`ClaimedInvite`] (the final chain plus any `remote_url`):

- audience-open: redelegates from the embedded ephemeral key to the redeemer,
  extending the chain by one hop.
- audience-scoped: verifies the chain's existing audience matches the redeemer
  and returns the chain as-is, erroring otherwise.

Persisting the returned chain and wiring up `remote_url` for sync are the
caller's responsibility. `From<Invite> for UcanProof` is also provided to
project the chain into dialog-ucan's authorization machinery.

```rust
use tonk_invite::{Invite, InviteAudience, DEFAULT_BASE_URL};

// Mint an audience-open invite and serialize it to a shareable link.
let invite = Invite::new(chain, InviteAudience::Open { seed }, remote_url).await?;
let url = invite.to_url(DEFAULT_BASE_URL)?;

// Redeem it on another client.
let claimed = Invite::parse_url(&url).await?.claim(&redeemer_did).await?;
assert_eq!(*claimed.chain.audience(), redeemer_did);
```

## Consumers

The `carry` CLI and the `tonk-ui` web client both depend on this crate so a
single invite URL format and claim flow is shared across them.
