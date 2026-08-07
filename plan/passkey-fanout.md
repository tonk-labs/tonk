# Passkey fan-out design spec

**Goal:** Let one account hold multiple passkeys as equal peers — enrolled through email + one-time code, or opportunistically from a credential that happens to be reachable — without coupling the account subject to any single passkey, without hiding authority state from the user, and without binding the account to Tonk's account service any harder than it is today.

**Approach:** Break the identity `account subject = HKDF(passkey PRF)`. Each passkey's PRF derives a *credential key*, one of many. The account subject becomes a fresh ephemeral root keypair that exists only during account genesis: it signs the repository descriptor, fans out subject-open delegations to the first credential key and to a recovery anchor (the account service's DID), and is then discarded. Later passkeys enroll by collecting delegations from every reachable anchor — a sibling credential, the recovery anchor, or both — so every credential ends up with at least one chain of uniform depth under a durable anchor. Peer equality for account-level acts (enrollment, revocation) is expressed in verification rules, not chain shape: any credential holding a currently-valid chain to the account subject may act for the account.

**Constraints:**

- The recovery anchor's authority must be visible in the delegation chain, revocable, and reproducible by a competing provider. Email + OTP is the service's *policy* for exercising a capability it visibly holds, never an out-of-band authority living only in its database.
- No state required to reconstruct or exercise account authority may exist only in tonk-run services. Chains, revocation artifacts, and the enrollment log live in the root-owned account repository; D1 and R2 are cache and index (survivability rule).
- The credential derivation itself (`derive.rs`, `tonk/root-key/v1`) is unchanged — only its *meaning* changes, from "the account subject" to "a credential key". Existing passkeys keep deriving the same key.
- Account delegations remain subject-open and audience-specific (`delegation.rs` shape). Space invites remain subject-specific; the two shapes must not blur (invite subject invariant).
- Existing v1 accounts (subject = first passkey's derived key) must keep working unmodified, and must be able to opt into fan-out without re-keying.
- The descriptor (`AccountRepositoryDescriptorV1`) is unchanged: it names a `did:key` subject and is signed once by that subject's key. Under fan-out the signature happens during genesis while the ephemeral root is alive.
- WebAuthn credentials are bound to `rp.id`. Portability lives entirely at the delegation layer: the account (subject DID, chains, repo) ports; individual passkeys never do.
- No speculative surface: this spec covers multiple passkeys and email-gated recovery. Paper keys, secondary providers, and social recovery must *fit* the model but are not built now.

## Terms

- **Credential key** — the Ed25519 key derived from one passkey's PRF output. Today this is called the root; after this change it is one credential among peers.
- **Account root** — a fresh Ed25519 keypair generated at account genesis and discarded before genesis completes. Its DID is the account subject forever.
- **Anchor** — an audience of a direct `root → x` delegation minted during genesis. Initially: the first credential key and the recovery anchor.
- **Recovery anchor** — the account service's own DID, holding a direct subject-open delegation from the root. A competing provider added later is just another anchor, reached through an existing chain rather than the root.
- **Sibling chain** — `root → credential_A → credential_B`: credential A enrolled credential B directly.
- **Anchor chain** — `root → recovery → credential_B`: the recovery anchor enrolled (or normalized) credential B.
- **Chain store** — a dumb map from audience DID to the delegations addressed to it. Holds only self-authenticating artifacts, so it can withhold but never forge. The account repository is the durable copy; the service's is a replica.
- **Claim** — "give me every delegation whose audience is this key". The one call a device with nothing but a freshly derived credential key can usefully make.

## Genesis ceremony

Replaces the current flow where `create_account` treats the passkey-derived signer as the subject.

1. Create the passkey; derive credential key `C1` from its PRF output.
2. Generate the ephemeral account root in the browser. It never leaves the ceremony scope.
3. Root signs, in order:
   - the account repository descriptor (unchanged v1 format; subject = root DID);
   - `root → C1`, subject-open, no expiry;
   - `root → recovery`, subject-open, no expiry, audience = the account service DID published at the descriptor remote.
4. Zeroize and drop the root.
5. Persist both delegations and the descriptor to the account repository genesis; publish both to the chain store keyed by audience; the service indexes email → subject.

**Anchor invariant.** Because the root is destroyed, every direct `root → x` anchor that will ever exist must be minted in step 3. After genesis, new anchors are only reachable by an existing credential delegating one (`Ca → competitor-recovery`), never by the root. An account that skips the recovery anchor at genesis can never be given one except through a live credential — and if all credentials are lost first, it is gone. This is the sharp edge storacha hit from the other direction (their space key survives but is often effectively lost, and "if a user doesn't have the private key for space, they cannot delegate access to a new email DID"). Genesis is therefore not the place to make the recovery anchor optional.

The `root → recovery` delegation is deliberately full (subject-open, empty command), not enroll-scoped: UCAN attenuation means an audience can only re-delegate what it holds, and recovery must be able to confer full account authority on a replacement credential. The containment is procedural and structural instead: the delegation is visible in the user's own repo, revocable by any peer credential, and the service's stated policy is to exercise it only for credential enrollment after email + OTP verification. A user who distrusts the service revokes that one link and (via any credential) delegates an equivalent anchor elsewhere.

## Enrollment ceremonies

Every enrollment produces a new credential key `Cn` from a new passkey's PRF and collects delegations from **every reachable anchor path**. A device stores all chains it holds; verifiers accept any one valid, unrevoked chain.

**Anchor path — the primary flow.** The user creates a passkey on the new platform, derives `Cn`, and asks the account service to enroll it. The service emails a one-time code; on verification it mints `recovery → Cn` under its `root → recovery` proof and publishes it to the chain store keyed by `Cn`. Chain: `root → recovery → Cn`, depth two regardless of enrollment order — this is what makes credentials effective peers for survivability.

Nothing else has to be present. A factory-fresh iPhone holding only the new passkey does: derive `Cn` from PRF → claim by `Cn`'s DID → receive `root → recovery → Cn`. No second device, no QR code, no out-of-band pairing.

Anchor-chain issuance is **always email + OTP gated**, even when the requester presents a currently-valid sibling chain. Rationale: an anchor chain outlives revocation of the enrolling credential. If issuance were gated only on "holds a valid chain", an attacker who briefly compromises one passkey could enroll a shadow credential, normalize it to an anchor chain, and survive the victim revoking the compromised passkey. Email gating puts a second factor and a notification in front of exactly that escalation. Every anchor issuance also notifies the account email, so the escalation is loud even when it succeeds.

The cost of making email primary, stated plainly: mailbox control becomes equivalent to account control. The containments are that the anchor link is visible and revocable by any peer, and that issuance is always announced.

**Sibling path — opportunistic, not a UX.** When a credential is reachable anyway during enrollment, `Ca` also mints `Ca → Cn`, subject-open, giving `root → Ca → Cn`. This is free, works offline, involves no service, and is sufficient authority on its own. It is not a flow the user is ever asked to perform: it is an extra chain the device keeps, and it is what gives `Cn` Peer/Delegated revocation authority over links an anchor chain alone would not cover. A credential holding only a sibling chain can request anchor normalization later through the same OTP gate.

Every enrollment appends a fact to the account repository's enrollment log: enrolled credential DID, enrolling authority (sibling DID or recovery), timestamp, and user-facing label. Sync-path verification never consults it — a presented chain stands on its own. Account-level acts do: revocation tooling walks it, and the seniority tie-break below reads its ordering. Whether that makes it an authority source for those acts is the one structural question still open (below).

## Chain store

The bootstrap problem is narrow and has one answer: a device that has just derived `Cn` and holds nothing else needs to *find* `root → recovery → Cn`. It cannot read the account repository, because the authority to sync that repository is the very thing it is looking for.

So the service exposes a store keyed by audience DID, with two operations — publish a delegation, and claim everything addressed to a key. This is deliberately not a credential registry and holds no policy:

- Entries are self-authenticating UCAN delegations. The store can withhold an entry; it cannot forge one, cannot alter one, and grants nothing by holding one.
- Claiming is unauthenticated by design. Knowing a credential's DID is not authority — the delegation is only usable by whoever holds that key.
- Every entry also lives in the account repository. A user, or a competing provider, can serve the same bytes; nothing here is reconstructible only from tonk-run state.

This is the piece taken directly from storacha's `access/delegate` / `access/claim`, and it is the whole of what the account service needs to do for enrollment beyond signing `recovery → Cn`.

## Verification

Unchanged in mechanism: a verifier accepts an invocation when it carries a valid delegation chain from the account subject to the invoker, with no chain link revoked. What changes is only that several chains may exist per credential and verifiers must accept any one of them. No keyring lookup, no repo-state dependency on the hot path.

## Revocation

`revocation.rs` artifacts already carry their own witness path and verify without a registry. Two authority classes exist today: **PathIssuer** (signer issued a delegation in the witnessed prefix) and **Delegated** (signer's authority flows through the target). Both break down under fan-out for the case that matters most — a peer revoking a link it does not depend on: Safari holding only `root → recovery → safari` has neither class of authority over `root → chrome`.

Add a third class:

- **Peer** — the signer presents any currently-valid delegation chain from the *same account subject* to itself. Any enrolled credential may revoke any delegation whose subject is the account.

This is the flat, equal-peers semantics, expressed where it belongs — in the verifier — rather than by contorting chain shape. The Delegated class's sharp edge (a downstream key revoking its own ancestor to lock others out) is inherited by Peer and accepted.

**Mutual revocation.** Two credentials revoking each other resolves by **seniority**: removals always win over concurrent non-removals, and between two removals the credential enrolled earlier survives. This is `@localfirst/auth`'s rule and it is better than the conservative "both revoked" reading — it avoids handing a briefly-compromised credential a mutual-destruction move, and it never orphans an account. The enrollment log already records the ordering the rule needs, which makes it the first thing in this design that reads the log (see the open question below). Recovery through email + OTP remains the arbiter of last resort when seniority is not enough.

Revoking a credential (the "lost Chrome passkey" flow, driven from any signed-in peer):

1. Walk the enrollment log for every delegation *issued by* the revoked credential and every credential enrolled through it that was later anchor-normalized.
2. Mint revocation artifacts for: the revoked credential's inbound links (`root → chrome`, any `x → chrome`), and — surfaced to the user for confirmation, not automatic — anything it enrolled.
3. Persist artifacts to the account repository and push to the service, which serves them to verifiers.
4. For each surviving credential holding only chains through the revoked link, prompt anchor normalization (email + OTP) so it regains a live chain.

Revoking the recovery anchor (`root → recovery`) is allowed under Peer authority and orphans nothing that holds a sibling chain — but it removes email recovery entirely until an equivalent anchor is delegated elsewhere. The UX must say so plainly.

## Recovery (all credentials lost)

1. User proves email control via OTP at the account service.
2. Service creates a passkey ceremony for a new credential key `Cr`, mints `recovery → Cr`, publishes it to the chain store keyed by `Cr`, records an enrollment-log fact marked `recovered`, and notifies the account email.
3. `Cr` now holds full authority; the user is prompted to review the credential list and revoke lost credentials (Peer authority).

There is no service-side custodial key beyond the `root → recovery` audience keypair the user already sees in their own chain. Losing email *and* all passkeys loses the account; that is the honest boundary of this design and must be stated at account creation.

## Storage and portability

- The account repository (root-owned, established at genesis) is the durable home of: the descriptor, all delegation chains, all revocation artifacts, and the enrollment log. Every linked device replicates it. Exporting the account is cloning the repo.
- The service's chain store and D1 email index are reconstructible caches. Anything present there but absent from the account repo is a bug — including the audience-keyed enrollment entries, which must be written to the repo by whichever credential first successfully claims them.
- Migrating to a competing provider: any credential delegates a new anchor (`Ca → competitor-recovery`, subject-open), the user enrolls a passkey under the competitor's RP through it, and optionally revokes `root → recovery`. The new anchor's chain is depth three rather than two; Peer verification makes depth irrelevant.

## Existing v1 accounts

A v1 account's subject *is* its passkey-derived key — which means, unlike the fan-out design, its root is still alive and re-derivable. That is a migration advantage:

- At any sign-in, the v1 credential can mint `subject → recovery` and enroll further passkeys as siblings, gaining everything in this spec except subject-rotation protection. No re-keying, no descriptor change; the subject key simply also remains a usable credential.
- What v1 accounts never gain: compromise of the *first* passkey is compromise of the subject itself, unrevocable by construction. A full re-key (new account, data migration) is the only cure and is out of scope here.
- Verifiers and ceremonies must therefore not assume the subject key is unreachable; they must accept both "subject signs directly" (v1) and "chain from subject" (fan-out) forever.

## Prior art

The closest system is storacha's w3up. It is worth being precise about what to take and what not to.

Their shape: a **space** `did:key` generated locally is the subject; an **account** `did:mailto` holds no key material at all and exists as a stable audience; an **agent** `did:key` per device. At space creation the space delegates full authority to the account. A new agent invokes `access/request`, the service emails a confirmation, and on approval issues both the account→agent delegation and a `ucan/attest` session; the agent then polls `access/claim` for everything addressed to it.

Take:

- **The email-gated issuance flow.** Confirmed as the ergonomic path, and it does not require a second device.
- **`access/delegate` / `access/claim`** — the audience-keyed chain store above.
- **Their failure mode as our invariant.** A space whose key is gone and which never got a recovery account is unrecoverable, with no way to add one after the fact.

Do not take:

- **Attestation rooted at the service.** `ucan/attest` carries `with: did:web:web3.storage` — the service's *own* DID. Verifiers must hardcode trust in it, and no link in the chain expresses that power, so no user can cut it. They need this only because `did:mailto` cannot sign: the account→agent delegation carries a placeholder signature and is worthless without the service vouching for it.

Our recovery anchor is a real keypair that got its authority from the account subject at genesis, so it simply *signs* `recovery → Cn`. Same UX, derived rather than inherent authority, visible as one revocable link, and no attestation concept or placeholder-signature machinery. That difference is the entire reason to keep the anchor as a delegation rather than adopting their session model wholesale.

Other systems informing this design: Fission/webnative and Keybase device provisioning (out-of-band pairing — rejected here on UX grounds, see deferred); KERI (self-certifying identifiers, append-only key event log, witnesses that hold receipts and no authority); `@localfirst/auth` (signature-chain membership, seniority tie-break); SPKI/SDSI certificate chain discovery (the store of self-authenticating certificates is a cache anyone can run — the argument that licenses the chain store above).

## Suggested landing order

Smallest stable slice first; each step is independently shippable and useful. Steps 1 and 2 are done; see Status.

1. **Peer revocation authority** in `revocation.rs` — self-contained, testable, needed by everything else, and immediately useful to v1 accounts.
2. **Multi-hop account grants** — a device may present `root → credential → device`. Prerequisite for any credential that is not the subject itself.
3. **Chain store** — publish and claim, keyed by audience DID. No policy, no new authority, and it is what removes the need for any pairing ceremony. Transport before policy.
4. **Email-gated anchor issuance** — `recovery → Cn` behind OTP, published to the store, notified to the account email. With 3, this is the whole enrollment flow for v1 accounts: the v1 subject key mints `subject → recovery` at sign-in, and every later passkey enrolls through the anchor.
5. **Ephemeral-root genesis** for new accounts, plus the enrollment log.
6. **Revocation walk UX** and the seniority tie-break, which is the first consumer of the enrollment log.

## Status

Landed on `feat/passkey-fanout`:

- **Step 1.** `RevocationAuthority::Peer` in `tonk-identity/src/revocation.rs`, plus
  `mint_peer_revocation`. Gated on the witnessed path being subject-open, so
  space invites keep the narrower classes.
- **Step 2.** `tonk-identity/src/credential.rs` mints the
  `credential → peer` enrollment link and composes chains under it;
  `delegation::validate_account_grant` replaces the five separate
  "exactly one proof" checks in the account service, worker and CLI, so a
  device may present `root → credential → device` at any depth. The
  registry now records the device's own hop rather than the chain root —
  identical for a one-hop grant, so no migration.

Next is the chain store (step 3), which is what closes the bootstrap gap: a
device holding only the second passkey currently has no way to obtain
`root → C2`.

Two decisions the implementation surfaced, both left open deliberately:

- **Peer at the account service's device-revocation gate.** `revoke_device`
  admits only `Delegated` (a device revoking itself) and `PathIssuer` (the
  account root). Admitting `Peer` would also let any *device* revoke any other
  device, because the service cannot tell a credential from a device — both
  hold subject-open chains from the root. That drops the current requirement
  that cross-device revocation re-derive the root from the passkey. Wants a
  decision before it is written.
- **Revocation artifacts are screened globally by target CID.** An artifact is
  authorized relative to its witnessed path, but `tonk-access-service` and the
  R2 key record only the target CID. Anyone who holds a delegation's bytes can
  mint `attacker → issuer`, prepend it to make the target sit at index 1, and
  qualify as `PathIssuer` — which globally revokes it. Reachable today by any
  invitee against their inviter's link. Predates this work and is untouched by
  it; subject-scoping the screen does not fix it, because subject-open grants
  are meant to apply across subjects.

## Open questions

- **Is the enrollment log an authority source?** This spec says no — verification consults only chains, so the hot path needs no registry. The price is that a credential and a device are structurally identical (both hold subject-open chains from the root), so no policy can express "credentials may revoke peers, devices may not"; that is exactly why the account service's device-revocation gate is stuck (above). KERI, Keybase and `@localfirst/auth` all go the other way and make the log authoritative for membership, accepting the lookup. The seniority tie-break already needs to read it. Decide this before step 5 — it is the one structural question left, and answering it "yes, for account-level acts only" would keep the sync hot path chain-only while unblocking the gate.
- Whether dialog-ucan policy predicates could scope `root → recovery` tighter than "full, by convention" without breaking the attenuation argument — revisit when policies land; do not block on it.
- Whether the enrollment log lives on the account repo's `main` alongside chains or in a reserved namespace — decide with the first implementation PR; the meta branch is local-only and cannot hold it.
- Whether the chain store should be readable without knowing a credential DID (e.g. "everything under this account subject"). Claiming by audience leaks nothing; enumerating by subject would expose the credential roster to anyone who knows the account DID.
- How many chains a device should retain. Minimum: its best anchor chain plus any sibling chain it depends on for Peer/Delegated revocation authority over links it may need to cut. Default to retaining everything; chains are small.

## Explicitly deferred

- **Out-of-band pairing** (QR / short code through a relay, as in Fission device linking and Keybase device provisioning). It is the dominant answer in the prior art and needs no service state at all, but it requires two devices in hand and is markedly worse UX than an emailed code. The opportunistic sibling path covers the case where a second credential is present anyway; a deliberate pairing ceremony is not built.
- **WebAuthn `largeBlob`.** Storing a credential's own chain inside the credential would make an enrolled passkey fully self-sufficient — one assertion returns both the PRF output and the chain, with no store and no network. Measured against this design's chains it fits comfortably: 280 bytes for one hop, 551 for two, 822 for three, against a 2 KB per-credential budget. Not built: blobs are only writable on an assertion *after* creation, so enrollment gains a ceremony step, and support is uneven (Safari/iOS 17+, Windows 11 only, varies by authenticator and unconfirmed for Google Password Manager). Revisit as an offline optimization once the chain store exists.
- Paper keys, secondary recovery providers, and social recovery — the anchor mechanism accommodates all three; none are built until there is an immediate use.
- Deliberate subject rotation (re-key ceremony) for v1 accounts.
- Attestation or provenance claims about where a passkey is stored (password manager vs platform); WebAuthn does not expose this reliably.
- Quorum rules for destructive account acts (N-of-M credentials to revoke the recovery anchor); single-credential Peer authority is accepted for now.
