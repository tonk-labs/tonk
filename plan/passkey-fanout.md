# Passkey fan-out design spec

**Goal:** Let one account hold multiple passkeys as equal peers — enrolled from a signed-in device or recovered through email + one-time code — without coupling the account subject to any single passkey, without hiding authority state from the user, and without binding the account to Tonk's account service any harder than it is today.

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

## Genesis ceremony

Replaces the current flow where `create_account` treats the passkey-derived signer as the subject.

1. Create the passkey; derive credential key `C1` from its PRF output.
2. Generate the ephemeral account root in the browser. It never leaves the ceremony scope.
3. Root signs, in order:
   - the account repository descriptor (unchanged v1 format; subject = root DID);
   - `root → C1`, subject-open, no expiry;
   - `root → recovery`, subject-open, no expiry, audience = the account service DID published at the descriptor remote.
4. Zeroize and drop the root.
5. Persist both delegations and the descriptor to the account repository genesis; the service stores its own copy of `root → recovery` and indexes email → subject.

The `root → recovery` delegation is deliberately full (subject-open, empty command), not enroll-scoped: UCAN attenuation means an audience can only re-delegate what it holds, and recovery must be able to confer full account authority on a replacement credential. The containment is procedural and structural instead: the delegation is visible in the user's own repo, revocable by any peer credential, and the service's stated policy is to exercise it only for credential enrollment after email + OTP verification. A user who distrusts the service revokes that one link and (via any credential) delegates an equivalent anchor elsewhere.

## Enrollment ceremonies

Every enrollment produces a new credential key `Cn` from a new passkey's PRF and collects delegations from **every reachable anchor path**. A device stores all chains it holds; verifiers accept any one valid, unrevoked chain.

**Sibling path (signed-in device present).** Credential `Ca` mints `Ca → Cn`, subject-open. Chain: `root → Ca → Cn` (or longer, through however `Ca` itself is anchored). Works offline, involves no service. This alone is sufficient authority.

**Anchor path (service reachable).** The service mints `recovery → Cn` under its `root → recovery` proof. Chain: `root → recovery → Cn`, depth two regardless of enrollment order — this is what makes credentials effective peers for survivability.

Anchor-chain issuance is **always email + OTP gated**, even when the requester presents a currently-valid sibling chain. Rationale: an anchor chain outlives revocation of the enrolling credential. If issuance were gated only on "holds a valid chain", an attacker who briefly compromises one passkey could enroll a shadow credential, normalize it to an anchor chain, and survive the victim revoking the compromised passkey. Email gating puts a second factor and a notification in front of exactly that escalation.

The standard ceremony when both paths are available: sibling delegation minted locally and stored immediately; anchor chain requested in the same flow, gated by OTP, stored when it arrives. A credential holding only a sibling chain can request anchor normalization at any later time through the same gate.

Every enrollment appends a fact to the account repository's enrollment log: enrolled credential DID, enrolling authority (sibling DID or recovery), timestamp, and user-facing label. The log is auditable data, not an authority source — verification never consults it — but revocation tooling walks it (below).

## Verification

Unchanged in mechanism: a verifier accepts an invocation when it carries a valid delegation chain from the account subject to the invoker, with no chain link revoked. What changes is only that several chains may exist per credential and verifiers must accept any one of them. No keyring lookup, no repo-state dependency on the hot path.

## Revocation

`revocation.rs` artifacts already carry their own witness path and verify without a registry. Two authority classes exist today: **PathIssuer** (signer issued a delegation in the witnessed prefix) and **Delegated** (signer's authority flows through the target). Both break down under fan-out for the case that matters most — a peer revoking a link it does not depend on: Safari holding only `root → recovery → safari` has neither class of authority over `root → chrome`.

Add a third class:

- **Peer** — the signer presents any currently-valid delegation chain from the *same account subject* to itself. Any enrolled credential may revoke any delegation whose subject is the account.

This is the flat, equal-peers semantics, expressed where it belongs — in the verifier — rather than by contorting chain shape. The Delegated class's sharp edge (a downstream key revoking its own ancestor to lock others out) is inherited by Peer and accepted: a revocation war between two credentials both claiming to be the user is resolved by the recovery flow (email + OTP re-enrollment), which is exactly the arbiter it should be.

Revoking a credential (the "lost Chrome passkey" flow, driven from any signed-in peer):

1. Walk the enrollment log for every delegation *issued by* the revoked credential and every credential enrolled through it that was later anchor-normalized.
2. Mint revocation artifacts for: the revoked credential's inbound links (`root → chrome`, any `x → chrome`), and — surfaced to the user for confirmation, not automatic — anything it enrolled.
3. Persist artifacts to the account repository and push to the service, which serves them to verifiers.
4. For each surviving credential holding only chains through the revoked link, prompt anchor normalization (email + OTP) so it regains a live chain.

Revoking the recovery anchor (`root → recovery`) is allowed under Peer authority and orphans nothing that holds a sibling chain — but it removes email recovery entirely until an equivalent anchor is delegated elsewhere. The UX must say so plainly.

## Recovery (all credentials lost)

1. User proves email control via OTP at the account service.
2. Service creates a passkey ceremony for a new credential key `Cr`, mints `recovery → Cr`, records an enrollment-log fact marked `recovered`, and notifies the account email.
3. `Cr` now holds full authority; the user is prompted to review the credential list and revoke lost credentials (Peer authority).

There is no service-side custodial key beyond the `root → recovery` audience keypair the user already sees in their own chain. Losing email *and* all passkeys loses the account; that is the honest boundary of this design and must be stated at account creation.

## Storage and portability

- The account repository (root-owned, established at genesis) is the durable home of: the descriptor, all delegation chains, all revocation artifacts, and the enrollment log. Every linked device replicates it. Exporting the account is cloning the repo.
- The service's R2 chain store and D1 email index are reconstructible caches. Anything present there but absent from the account repo is a bug.
- Migrating to a competing provider: any credential delegates a new anchor (`Ca → competitor-recovery`, subject-open), the user enrolls a passkey under the competitor's RP through it, and optionally revokes `root → recovery`. The new anchor's chain is depth three rather than two; Peer verification makes depth irrelevant.

## Existing v1 accounts

A v1 account's subject *is* its passkey-derived key — which means, unlike the fan-out design, its root is still alive and re-derivable. That is a migration advantage:

- At any sign-in, the v1 credential can mint `subject → recovery` and enroll further passkeys as siblings, gaining everything in this spec except subject-rotation protection. No re-keying, no descriptor change; the subject key simply also remains a usable credential.
- What v1 accounts never gain: compromise of the *first* passkey is compromise of the subject itself, unrevocable by construction. A full re-key (new account, data migration) is the only cure and is out of scope here.
- Verifiers and ceremonies must therefore not assume the subject key is unreachable; they must accept both "subject signs directly" (v1) and "chain from subject" (fan-out) forever.

## Suggested landing order

Smallest stable slice first; each step is independently shippable and useful.

1. **Peer revocation authority** in `revocation.rs` — self-contained, testable, needed by everything else, and immediately useful to v1 accounts.
2. **Sibling enrollment** — multi-passkey for v1 accounts (subject key mints sibling delegations directly). Fixes the Chrome-on-Mac / Safari-on-iOS split with no service or genesis change.
3. **Ephemeral-root genesis + recovery anchor** for new accounts, plus the enrollment log.
4. **Anchor normalization and email-gated recovery** in the account service.
5. **v1 opt-in fan-out** (`subject → recovery` minting at sign-in) and the revocation walk UX.

## Status

Landed on `feat/passkey-fanout`:

- **Step 1.** `RevocationAuthority::Peer` in `tonk-identity/src/revocation.rs`, plus
  `mint_peer_revocation`. Gated on the witnessed path being subject-open, so
  space invites keep the narrower classes.
- **Step 2, enabling half.** `tonk-identity/src/credential.rs` mints the
  `credential → peer` enrollment link and composes chains under it;
  `delegation::validate_account_grant` replaces the five separate
  "exactly one proof" checks in the account service, worker and CLI, so a
  device may present `root → credential → device` at any depth. The
  registry now records the device's own hop rather than the chain root —
  identical for a one-hop grant, so no migration.

Not started: the browser enrollment ceremony and wherever the enrollment link
is served from (a device holding only the second passkey has no way to obtain
`root → C2` today), and steps 3–5.

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

- Whether dialog-ucan policy predicates could scope `root → recovery` tighter than "full, by convention" without breaking the attenuation argument — revisit when policies land; do not block on it.
- Concurrent revocation races (two credentials revoking each other) resolve through recovery, but the verifier-visible interim state needs defining: likely "both revoked until re-enrollment", the conservative reading.
- Whether the enrollment log lives on the account repo's `main` alongside chains or in a reserved namespace — decide with the first implementation PR; the meta branch is local-only and cannot hold it.
- How many chains a device should retain. Minimum: its best anchor chain plus any sibling chain it depends on for Peer/Delegated revocation authority over links it may need to cut. Default to retaining everything; chains are small.

## Explicitly deferred

- Paper keys, secondary recovery providers, and social recovery — the anchor mechanism accommodates all three; none are built until there is an immediate use.
- Deliberate subject rotation (re-key ceremony) for v1 accounts.
- Attestation or provenance claims about where a passkey is stored (password manager vs platform); WebAuthn does not expose this reliably.
- Quorum rules for destructive account acts (N-of-M credentials to revoke the recovery anchor); single-credential Peer authority is accepted for now.
