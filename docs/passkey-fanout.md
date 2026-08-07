# Passkey fan-out (V1)

One account, several passkeys, none of them the account.

A passkey is bound to one authenticator, so an account whose identity *is* a
passkey reaches only the platforms that authenticator reaches. Chrome on a Mac
and Safari on an iPhone are routinely not the same set. This specification
breaks the identity `account subject = HKDF(passkey PRF)` without coupling the
account to any single passkey, without hiding authority state from the user,
and without binding the account to Tonk's account service any harder than it
is bound today.

## Model

- **Account subject** — an Ed25519 keypair generated during genesis and
  destroyed before genesis completes. Its DID names the account forever.
  Nothing holds the key afterwards.
- **Credential key** — the key derived from one passkey's PRF output. The
  derivation (`tonk/root-key/v1`) is unchanged; its meaning changes from "the
  account" to "one credential among peers".
- **Device key** — a per-device key that acts through a credential.
- **Anchor** — the audience of a direct `subject → x` delegation minted at
  genesis. Initially the first credential and the recovery anchor.
- **Recovery anchor** — a provider's own keypair, holding a direct
  subject-open delegation. A competing provider added later is another
  anchor, reached through a credential rather than through the subject.

Account authority is carried by **subject-open, audience-specific, command-open**
delegations. Space invites are subject-specific, and the two shapes must not
blur: a chain that is subject-specific is not account authority, whatever else
is true of it.

Every credential reaches the subject through a chain of such delegations, and
every device reaches it through a credential. Chain depth is not fixed and
carries no meaning. `subject → C1 → device`,
`subject → recovery → C2 → device`, and longer, are equally valid. A principal
may hold several chains at once and a verifier accepts any one of them.

## Genesis

The account subject signs, in order, and is then destroyed:

1. the account repository descriptor, naming itself as subject;
2. `subject → C1`, to the first passkey's credential key;
3. `subject → recovery`, to the recovery anchor's DID, which the provider
   publishes.

`subject → recovery` is deliberately full rather than enroll-scoped. UCAN
attenuation means an audience can only re-delegate what it holds, and recovery
must be able to confer full account authority on a replacement credential. The
containment is structural rather than syntactic: the delegation is one visible
link in the user's own chain, revocable by any peer.

**Anchor invariant.** Because the subject is destroyed, every direct
`subject → x` delegation that will ever exist is minted here. Afterwards a new
anchor is reachable only by an existing credential delegating one. An account
that leaves genesis without a recovery anchor can never be given one except
through a live credential, and if every credential is lost first, the account
is gone.

## Enrollment

A credential is enrolled by receiving a subject-open delegation. Two paths
produce one; a credential may end up holding chains from both, and should
retain all of them.

**Anchor path.** The provider verifies a one-time code sent to the account
address, mints `recovery → Cn` under its genesis proof, publishes it, and
mails the account address to say that it did. This requires nothing but the
new passkey: no second device, no pairing ceremony. It produces a chain of
depth two regardless of enrollment order, which is what makes credentials
effective peers for survivability.

**Sibling path.** A reachable credential mints `Ca → Cn` locally. No provider,
works offline, and is sufficient authority on its own. It is not a flow a
person is asked to perform — it is an extra chain a device keeps when a
credential happened to be present, and it is what gives `Cn` revocation
authority over links an anchor chain alone would not cover.

Anchor issuance is gated on the code **even when the requester already holds a
valid chain**. An anchor chain outlives revocation of whatever enrolled it, so
gating on "holds a valid chain" would let an attacker who briefly compromises
one passkey enrol a shadow credential, normalize it to an anchor chain, and
survive the victim revoking the compromised passkey. The code is a second
factor in front of exactly that escalation; the notice makes the escalation
loud even when it succeeds.

The cost, stated rather than mitigated away: mailbox control becomes account
control. The containments are that the anchor link is visible and revocable by
any peer, and that every issuance is announced.

## Chain store

A device that has just derived `Cn` holds nothing else. It does not know the
account subject, holds no delegation, and cannot read the account repository,
because the authority to sync that repository is the thing it is looking for.
The only question it can ask is what has been addressed to the one key it has.

A provider therefore offers a store keyed by audience DID, with two
operations:

- **publish** a signed delegation chain;
- **claim** every chain addressed to a given key.

Both are unauthenticated, and this is not a concession. The entries are
self-authenticating UCAN delegations: publishing one confers nothing, and
knowing a DID is not authority, since a delegation is useless without the
matching private key. A store can withhold an entry. It cannot forge one,
alter one, or grant anything by holding one.

Every entry also belongs in the account repository, so a user or a competing
provider can serve the same bytes. Anything present in a provider's store and
absent from the account repository is a bug.

A provider may additionally require that it hosts the account a published
chain runs from. That is abuse control, not authority — the chain itself
already settles who may enrol — and it keeps an open write endpoint from being
open storage.

Entries are enumerable per credential and never per account: given an account
subject it must not be possible to list its credentials.

## Verification

A verifier accepts an invocation carrying a valid delegation chain from the
account subject to the invoker, with no link revoked. Several chains may exist
per credential and a verifier must accept any one of them. No keyring lookup
and no repository state on the hot path.

Verifiers must not assume the subject key is unreachable. Both "the subject
signs directly" and "a chain from the subject" remain valid indefinitely, for
the sake of accounts predating this design.

## Revocation

Revocation artifacts carry their own witness path and verify without a
registry. Two authority classes exist: **PathIssuer**, where the signer issued
a delegation in the witnessed prefix, and **Delegated**, where the signer's
authority flows through the target. Neither covers the case that matters most
here — a peer withdrawing a link it does not depend on. A credential holding
only `subject → recovery → safari` has no standing over `subject → chrome`
under either.

A third class:

- **Peer** — the signer presents its own currently-valid chain from the same
  subject-open authority the target answers to. Any enrolled credential may
  revoke any delegation whose subject is the account.

This is flat, equal-peers semantics expressed in the verifier rather than by
contorting chain shape. It is gated on the witnessed path being subject-open,
so space invites keep the narrower classes.

The Delegated class's sharp edge — a downstream key revoking its own ancestor
to lock others out — is inherited by Peer and accepted. **Mutual revocation**
resolves by seniority: removals win over concurrent non-removals, and between
two removals the credential enrolled earlier survives. Recovery through the
anchor path remains the arbiter of last resort.

Revoking a credential means withdrawing its inbound links, and surfacing
anything it enrolled for the user to confirm rather than cutting it
automatically. Surviving credentials left holding only chains through a
revoked link need anchor normalization to regain a live one.

Revoking `subject → recovery` is permitted under Peer authority. It orphans
nothing that holds a sibling chain, but it removes the anchor path entirely
until an equivalent anchor is delegated elsewhere, and a user must be told so
plainly.

## Recovery

With every credential lost, a user proves control of the account address, the
provider mints `recovery → Cr` for a fresh credential and publishes it, and
`Cr` holds full account authority. The user is then prompted to review the
credential list and revoke what was lost.

There is no provider-side key beyond the anchor keypair the user already sees
in their own chain. Losing the address *and* every passkey loses the account.
That is the honest boundary of this design and belongs in front of a person at
account creation.

## Storage and portability

The account repository, established at genesis, is the durable home of the
descriptor, every delegation chain, every revocation artifact, and the
enrollment record. Every linked device replicates it; exporting an account is
cloning it. A provider's stores are reconstructible caches.

Migrating to a competing provider: any credential delegates a new anchor
(`Ca → competitor-recovery`, subject-open), the user enrols a passkey under
the competitor's relying party through it, and optionally revokes the original
anchor. The new anchor's chains are one hop deeper, and depth is irrelevant to
verification.

WebAuthn credentials are bound to a relying party. Portability lives entirely
at the delegation layer: the account ports, individual passkeys never do.

## Existing accounts

An account whose subject *is* its first passkey's derived key still has that
key, and can adopt everything here without re-keying. At any sign-in it may
mint `subject → recovery` and enrol further passkeys as siblings; the subject
key simply also remains a usable credential.

What such an account never gains is subject-rotation protection: compromise of
the first passkey is compromise of the subject itself, unrevocable by
construction. A full re-key is the only cure and is out of scope.

## Rationale

The closest prior system is storacha's w3up: a locally generated space
`did:key` as subject, a keyless `did:mailto` account as a stable audience, and
per-device agents, with new agents authorized by email and served their
delegations from a store keyed by audience. The audience-keyed store and the
email-gated flow are taken directly from it, and its failure mode — a space
whose key is gone and which never received a recovery account is
unrecoverable, with no way to add one afterwards — is the anchor invariant
above.

What is deliberately not taken is the shape of its authority. Because
`did:mailto` cannot sign, an account→agent delegation there carries a
placeholder signature and is worthless without a `ucan/attest` from the
service, rooted at the *service's own* DID. Verifiers must trust that DID out
of band and no link in the chain expresses the power, so no user can decline
it. A recovery anchor holding a real key needs none of that: it signs
directly, its standing comes from one delegation the account made, and any
peer can cut it. Same ergonomics, derived rather than inherent authority.

Other systems informing this design: Fission/webnative and Keybase device
provisioning, whose out-of-band pairing needs no service state but needs two
devices in hand; KERI, for self-certifying identifiers whose key state lives
in a controller-owned log with witnesses that hold receipts and no authority;
`@localfirst/auth`, for signature-chain membership and the seniority rule
above; and SPKI/SDSI certificate chain discovery, for the argument that a
store of self-authenticating certificates is a cache anyone can run.

## Deferred

- **Out-of-band pairing** (QR or short code through a relay). It needs no
  provider state at all, but it needs two devices in hand and is markedly
  worse than an emailed code. The sibling path covers the case where a
  credential is present anyway.
- **`largeBlob`.** Storing a credential's own chain inside the credential
  would make an enrolled passkey self-sufficient — one assertion yielding both
  the PRF output and the chain, with no store and no network. Chains fit
  comfortably: 280 bytes for one hop, 551 for two, 822 for three, against a
  2 KB per-credential budget. Support is uneven and blobs are only writable on
  an assertion after creation, so this is an offline optimization rather than
  a mechanism.
- Paper keys, secondary recovery providers, and social recovery. The anchor
  mechanism accommodates all three.
- Deliberate subject rotation for accounts predating this design.
- Attestation about where a passkey is stored; WebAuthn does not expose it
  reliably.
- Quorum rules for destructive account acts. Single-credential Peer authority
  is accepted.

## Open questions

- **Is the enrollment record an authority source?** This specification says
  no for the sync path: a presented chain stands on its own and verification
  consults nothing else. The price is that a credential and a device are
  structurally identical — both hold subject-open chains from the subject — so
  no policy can express "credentials may revoke peers, devices may not". The
  seniority rule above already needs to read enrollment order. KERI, Keybase
  and `@localfirst/auth` all make the log authoritative for membership and
  accept the lookup.
- Whether policy predicates could scope `subject → recovery` tighter than
  "full, by convention" without breaking the attenuation argument.
- Where the enrollment record lives within the account repository. A
  local-only branch cannot hold it.
- How many chains a device should retain. Minimum is its best anchor chain
  plus any sibling chain it needs for revocation authority over links it may
  have to cut; chains are small enough to default to keeping everything.
