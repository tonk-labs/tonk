# Revocations at `/ucan/`

Status: draft for discussion
Scope: `tonk-access-service` presign path and revocation storage; deletes the R2 revocation registry, the account service's `/revocations` relay, and `REVOCATION_RELAY_URL`
Sequenced first: decommissioning the account service (`plan/account-service-decommission.md`, not yet written) is blocked on this, and `/customer/purge` is blocked on that.

## 1. What exists now

A revocation is an immutable artifact published to an R2 bucket under `revocations/{target}/{artifact}`. Publication goes through the account service's `/revocations` relay, named per-remote by `REVOCATION_RELAY_URL`; a mint refuses if its remote has no relay configured.

On the read side, every presign at `POST /ucan/` calls `revocation::assess`, which:

1. Collects every CID presented in the container
2. **Lists the R2 prefix to completion**, fetching bytes for artifacts not already in an isolate-local snapshot
3. Verifies each new artifact and unions its target into the snapshot
4. Answers a [`SetVerdict`]

The snapshot is monotone and isolate-local, with a 60s freshness TTL (`REVOCATION_TTL_MS`) and a 10-minute grace window (`REVOCATION_GRACE_MS`) for when the listing fails.

That last part is where the complexity lives. `SetVerdict` has four arms, two of which exist only to describe degrees of not-knowing:

```rust
Allowed,                  // fresh snapshot, nothing revoked
AllowedStale(String),     // listing failed, prior clean snapshot inside grace
Revoked,
Unavailable(String),      // no snapshot can safely clear this request
```

`AllowedStale` and `Unavailable` are both "the registry did not answer." The 10-minute grace is a guess at how long serving a possibly-stale verdict is preferable to refusing traffic, and the freshness counter must not advance on an invalid artifact or the grace window silently extends past a bad refresh.

### What this costs

An R2 **LIST** on the hot path of every presign. Not a point lookup: a prefix listing that must run to its final page to count as complete, because a partial listing cannot prove absence. Cost grows with the number of revocations ever published, and it is paid by every request whether or not anything was ever revoked.

### Version alignment

We are already on `1.0.0-rc.1`: `dialog-ucan-core` tags envelopes `ucan/dlg/1.0.0-rc.1` and `ucan/inv/1.0.0-rc.1`, which is exactly what the revocation spec's own example carries. So this is not adopting a new model or getting ahead of a standard — it is implementing the revocation half of a spec family whose delegation and invocation halves we already produce and verify. A `ucan/revoke` invocation is an ordinary invocation in the envelope format we ship today.

## 2. What changes

Two independent moves. Either could ship without the other, but together they delete the subsystem.

### 2.1 Revocation becomes an invocation

A revocation arrives at `POST /ucan/` like everything else. The wire shape is the spec's ([v1.0.0-rc.1](https://github.com/ucan-wg/revocation)), not one of ours:

```json
{
  "iss": "did:plc:...",
  "sub": "did:key:...",
  "do": "ucan/revoke",
  "args": {
    "revoke": { "/": "bafkre...target delegation CID" },
    "path":   [{ "/": "bafkre..." }, { "/": "bafkre..." }]
  },
  "nonce": { "/": { "bytes": "" } },
  "prf": [{ "/": "bafkr4..." }]
}
```

It is an ordinary invocation with `sub`, `iss`, and `prf`, so it lands on the subject whose consumer it concerns and is metered and screened like anything else. `revoke` names the target delegation by canonical CID; `path` is the optional delegation-path witness (§2.5). The nonce is empty by spec because revocation is idempotent, which is worth keeping: replaying one is a no-op rather than a second billable act.

**Who may revoke.** The spec:

> An Issuer of a particular Delegation in a proof chain MAY revoke that Delegation. Note that this is not always the same as revoking the Delegation they they Issued; any UCAN that contains a proof where the revoker matches the `iss` field — even transitively in the delegation chain — MAY be revoked.

Any issuer in the chain, transitively. Revocation authority may also itself be delegated (`can: "ucan/revoke"` with the target CID in `args`), so a principal outside the chain can hold it — this is the case storacha's `scope` field was invented to cover, and the spec handles it with a delegation instead of a store field.

**Validation is the spec's pseudocode, and it is a two-part check:**

```js
const delegators = invocation.prf.map(proof => proof.iss)

invocation.prf.forEach(delegation => {
  store.lookup(delegation).then(revocation => {
    // Is the revocation issuer in this proof chain?
    if (delegators.includes(revocation.iss)) { throw ... }
    // Is the revocation based on a delegated revocation?
    const cids = revocation.iff.filter(cav => !!cav.rev)
    if (cids.length === 1 && invocation.prf.includes(cids[0])) { throw ... }
  })
})
```

So the store cannot hold bare CIDs: matching requires the revocation's **issuer** (to test chain membership) and enough of it to spot a delegated revocation. The stored value is the revocation, not just the fact that one exists.

Consequences:

- **Metered.** The subject is an active consumer, so the invocation counts under their usage like any other. Revoking is not free, which is correct: it is a write against their namespace.
- **Gated.** It rides the provisioning gate. An unactivated customer cannot revoke, which is consistent with an unactivated customer not being served at all.
- **The relay disappears.** `/revocations` on the account service, `REVOCATION_RELAY_URL`, and the mint-time refusal for a relay-less remote all go. A revocation goes where every other invocation goes.

### 2.2 The registry becomes KV

Storage moves from an R2 prefix to KV, keyed for point lookup rather than listing.

**Key shape.** The spec settles this. It defines the store as a per-subject cache of revoked CIDs, and validation as plain set membership:

> The Agent that controls a resource MUST maintain a cache of Revocations for which it is the Subject.
>
> During validation of a UCAN delegation chain, the canonical CID of each UCAN delegation MUST be checked against the cache. If there's a match, the relevant Delegation MUST be ignored.

So:

```
revoked:{target_cid}/{subject_did}  →  ""
```

One key per `(target, subject)` fact, value empty: the key *is* the fact.

**Why not a set as the value.** KV has no compare-and-swap and no atomic read-modify-write; `put` replaces the whole value. Two principals revoking the same delegation concurrently would both read the set, both write their own singleton, and one revocation would be silently lost. Neither writer sees a conflict, so no retry can recover it. Losing a revocation because a write swapped it is not a failure mode a security primitive gets to have.

Key-per-pair has no such window: distinct keys never collide, so writes need no read, no merge, and no retry.

Keyed by the revoked delegation's canonical CID, valued by the set of DIDs that revoked it. Both halves are forced by the spec's validation:

```js
const delegators = invocation.prf.map(proof => proof.iss)
if (delegators.includes(revocation.iss)) { throw ... }
```

The lookup is by target CID, and the match tests whether the *revoker* appears among the issuers of the chain being presented. A bare "this CID is revoked" cannot answer that, so the issuer has to be in the value.

**What is stored is an authority fact, not a scoped effect.** The spec:

> An Issuer of a particular Delegation in a proof chain MAY revoke that Delegation. Note that this is not always the same as revoking the Delegation they they Issued; any UCAN that contains a proof where the revoker matches the `iss` field — even transitively in the delegation chain — MAY be revoked.

A revoker's reach is the whole subtree beneath any delegation they issued, not just the edge they signed: the root can revoke a grandchild directly. So `(revoker, target)` records a standing statement — *this principal revoked this delegation* — and carries no notion of which chain it applies to.

The validator's chain check is then not a scoping rule but a confirmation that the stored revoker actually had authority over *this* path. Where they did not, the entry is irrelevant here rather than narrowed:

```
a1 → b → c     revocation of b by a1 applies: a1 ∈ {a1, b}
a2 → b → c     same entry is irrelevant: a1 ∉ {a2, b}
```

`a1` never held authority over the second path, so their revocation was never about it. Matching is by DID, so one principal issuing several delegations that reach the same target revokes through all of them at once.

This is why the store cannot be a flat revoked-CID set, and it is the same fact ucanto carries as `scope`.

> **Bind on `sub`, not `iss`.** The spec's pseudocode tests `delegators.includes(revocation.iss)`, which contradicts its own "Delegating Revocation" section. When Alice delegates `ucan/revoke` to Zelda, Zelda's invocation carries `iss: Zelda, sub: Alice`; `delegators` contains only delegation-chain issuers, so Zelda is never in it and a delegated revocation could never match. The subject is the principal whose authority is exercised, and that is who must be in the chain.
>
> Fixed upstream in [ucan-wg/revocation#4](https://github.com/ucan-wg/revocation/pull/4), open at time of writing. We implement the corrected form.
>
> Our `tonk_identity::revocation::verify` currently returns `issuer`, and reaches the delegated case through a separate branch rather than by subject. That field becomes the subject.

**Reads are point gets, not listings.** Both halves of the key are known before reading: the CIDs come from the presented chain, the subjects are the issuers that chain proves. So the exact keys can be computed and fetched.

That is also the sharper question. Listing answers "who revoked this target," and the caller then discards every subject that is not in the chain. Fetching `(cid, subject)` pairs asks "did *this* principal revoke it," which is what the verdict actually turns on.

The product stays small because chains here are short: root to device is 2 CIDs by 2 issuers, so four reads, almost always four misses. That is nothing like the R2 listing it replaces, whose cost grew with every revocation ever published.

`RevocationIndex::subjects` remains for the general case, and `revoked_by_any` is the presign path's query. The default implementation of the latter lists and intersects, which is correct for any backend; the KV implementation overrides it with point reads.

> ucanto stores the same information as an explicit `scope` set per revoked CID. That implementation is what this spec was written from, so the agreement is not a coincidence: `scope` and the spec's `revocation.iss` are the same fact under different names. Where they differ is out-of-chain revokers, which ucanto models with `scope` and the spec models with a delegation of `ucan/revoke`.

**Eventual consistency is what the spec assumes.** It is written for the weakest case and says so directly:

> UCAN revocation MAY operate in fully eventually consistent contexts, with single sources of truth, or among nodes participating in consensus. The format of the revocation does not change in these situations.

KV propagates globally in roughly 60 seconds, and the current design is *already* eventually consistent by the same order of magnitude (60s snapshot TTL plus a 10-minute grace window). Revocation is a "stop future use" primitive, not a real-time kill switch.

One thing to carry over: the spec RECOMMENDS accepting a revocation for a delegation the store has not seen yet, precisely because a malicious holder may sit on a capability and reveal it late. So the write path must not require the target to be known.

**The stale/grace machinery disappears.** A KV miss is a definite "not revoked," not an ambiguous outage, so `SetVerdict` collapses to two arms:

```rust
Allowed,
Revoked,
```

No `AllowedStale`, no `Unavailable`, no freshness counter, no grace window, no monotone isolate snapshot. A KV read error is the service's own unavailability — the same 503 the provisioning gate already answers with — rather than a third verdict the caller must reason about.

**Likely faster, not slower.** Replacing an R2 prefix listing with a KV point read should reduce hot-path latency: KV is cache-backed at the edge and the read is O(1) in the number of revocations, where the listing is O(n).

> The write path is still last-write-wins per key, so two principals revoking the same CID concurrently can lose a scope. Storacha sidesteps this with an attribute-level `UpdateItem`, which KV has no equivalent for. Either read-modify-write with a retry, or split to `revoked:{cid}:{scope}` and accept a listing per CID. Needs an answer before it ships.

### 2.3 Who may write: a consumer-row check, not the billing gate

The provisioning gate asks "is someone paying for this subject." That is the wrong question for a revocation. The right one is "do we hold anything this revocation could protect," which the consumer row already answers:

| Consumer row | Verdict | Why |
|---|---|---|
| Absent | Deny, not a registered consumer | Nothing here to protect; an open write surface otherwise |
| `deletion_state = Deleted` | Deny, pruned | The replica is gone, so the revocation guards nothing |
| Present, any customer status | Accept | We hold data; a lapsed bill is not a reason to refuse a safety mechanism |

So a `Registered`, `Suspended`, or over-limit customer can still revoke. Revocation is the response to a compromised key or a rogue agent, and that is exactly the moment someone might also be behind on billing. What it is *not* is an unbounded write surface, because a subject we never registered is refused outright.

### 2.4 Per-delegation, and a revoked proof is not a refusal

Two distinct reasons a stored revocation may not stop a request, worth keeping separate because they are different mechanisms:

1. **The revoker is not in this chain.** `delegators.includes(revocation.iss)` fails, so the revocation does not apply here at all — the `a2 → b → c` case above. The chain still runs *through* the revoked delegation.
2. **A path exists that avoids the target.** The spec's own framing:

   > Revocation of a particular proof does not guarantee that the Agent can no longer access to the capability in question. If an Agent is able to construct a valid proof chain without relying on the revoked proof, they still have access.

Either way the validator ignores the revoked delegation and keeps going, refusing only when no path clears. Today's `assess` answers over the union of every presented CID and can express neither.

There is **no partial or capability-subset revocation** in the spec. You revoke a delegation instance; to narrow authority you re-delegate something narrower and revoke the wide one.

### 2.5 Processing a revocation

The container arrives at `/ucan/` like any other, carrying the revoked delegation and every proof in the evidence path as blocks.

1. **Already recorded?** If `(target_cid, issuer)` is already in the set, answer Ok and stop. Idempotent by spec — the empty nonce says so — and a replay must not bill twice.
2. **Is the subject ours to protect?** Consumer-row check per §2.3. Absent or pruned, refuse and say which.
3. **Is the evidence good?** The revoker must be an issuer in the witnessed path through the target, or hold a delegation of `ucan/revoke` for it. `tonk_identity::revocation::verify` already draws exactly this distinction as `RevocationAuthority::{PathIssuer, Delegated}`.
4. **Record and answer.** Add the issuer to the target's set. On invalid evidence, refuse with the reason.

Note the target need not be known to us. The spec RECOMMENDS accepting revocations for delegations not yet seen, since a holder may sit on a capability and reveal it late, so step 3 verifies the evidence rather than looking the target up.

### 2.6 Immutable, monotone, evictable on expiry

Three properties the spec fixes, all of which make the store cheap:

> Revocations MUST be immutable and irreversible.
>
> Recipients of revocations SHOULD treat them as a monotonically-growing set.
>
> Revocations MAY be evicted once the UCAN that they reference expires or otherwise becomes invalid through its proactive mechanisms, such as expiry (`exp`) plus some clock-skew buffer.

Immutable and monotone mean a revocation is only ever added, never updated or withdrawn, so the write path is an insert and there is no reconciliation. Un-revoking is not a thing: the spec says issue a fresh delegation instead. And eviction on target expiry is what keeps a per-subject value from growing without bound, since our delegations carry `exp`.

### 2.7 Deriving the paths, without touching dialog

The check needs the chain's structure, not just its CIDs: for each authorizing path, the ordered `(cid, issuer)` pairs whose issuers form the `delegators` set. Today's `collect_presented` flattens every delegation into one `BTreeSet<String>`, which cannot express a path at all, so none of §2.4's cases are decidable from it.

`UcanAuthorizer::authorize` proves the chain and returns only a `Permit`, dropping the structure. But nothing is lost: `InvocationChain` carries `delegations: HashMap<Cid, Arc<Delegation>>`, the entire proof set, and we hold the same container bytes. So after authorization succeeds we rebuild the chain and walk it:

1. Start at `invocation.proofs()`
2. Follow each delegation's own proofs through the map, collecting `(cid, issuer)`
3. Each walk yields one authorizing path; its issuers are that path's `delegators`

Re-parsing costs a little, and `authorize()` has already proven the chain, so the walk is pure structure with no re-verification. Crucially this needs **no change to dialog**: an earlier draft proposed adding a `validateAuthorization`-style hook to the authorizer, ucanto-style, which would have meant a dialog PR and a release to consume it.

> The one thing the hook would buy that this does not: ucanto's validator calls the revocation check per candidate chain and, on a revoked one, keeps trying other candidates. Walking after the fact sees the path the authorizer settled on, not every path it might have taken. For our chains (root → device, occasionally one more) alternatives are rare, and the "routes around the target" case in §4 pins the behaviour we do support. Worth revisiting if chains ever branch.

## 3. What gets deleted

- `rust/tonk-access-service/src/revocation.rs` — the snapshot, TTL, grace, `SetVerdict`'s two unknowable arms, `RevocationSource`, `StoredArtifact` (365 lines)
- `rust/tonk-access-service/src/revocation/r2.rs` — the R2 source (84 lines)
- `/revocations` on the account service, and the `REVOCATIONS` R2 bucket binding
- `REVOCATION_RELAY_URL` from every wrangler environment, and the mint-time refusal that depends on it

`rust/tonk-identity/src/revocation.rs` (minting and verification) mostly stays: the artifact still has to be signed and verified. What changes is where it is sent and how the result is stored.

## 4. Test matrix

Each row is a case discussed while designing this, and several exist to pin a distinction that a simpler store shape would silently get wrong.

### Authority

| Case | Expected |
|---|---|
| Direct issuer revokes the delegation it signed | Revoked |
| Root revokes a grandchild it never signed | Revoked (authority is the subtree) |
| Delegated revoker: `iss: Zelda, sub: Alice` | Revoked ([PR #4](https://github.com/ucan-wg/revocation/pull/4); fails under the unpatched pseudocode) |
| Revoker outside the delegation network (Mallory) | Refused, unauthorized |
| `path` witness absent | Refused |
| `path` whose signatures do not verify | Refused |
| Target CID absent from its own witness path | Refused |

### Which paths a revocation reaches

The discriminating cases. One target, one stored entry, different verdicts per path:

```
a ──a1──► b ──► c     revoked   (a ∈ {a, b})
a ──a2──► b ──► c     revoked   (a ∈ {a, b})
k ──k1──► b ──► c     stands    (a ∉ {k, b})
```

| Case | Expected |
|---|---|
| One DID issues two delegations reaching the target; revocation names that DID | Both paths revoked, and a third path under a different issuer still authorizes |
| Two distinct issuer DIDs, revocation names one | Only that issuer's path is revoked |
| A path that routes around the target entirely | Authorized (the "two tickets" rule) |

The first row is what forces the value to be a set of DIDs. Keyed by delegation instance, `a2` would wrongly survive; ignoring the subject, `k1` would wrongly die.

### Consumer row (§2.3)

| Row state | Expected |
|---|---|
| Absent | Refused, not a registered consumer |
| `deletion_state = Deleted` | Refused, pruned |
| Present, customer `Registered` / `Suspended` / over limit | Accepted |

### Store behaviour

| Case | Expected |
|---|---|
| Replay of the same `(target, subject)` | Ok, idempotent, not billed twice |
| Two different subjects revoke one target | Both recorded, either matches its own chains |
| Revoking a target we have never seen | Accepted (late-reveal rule) |
| KV read fails during validation | 503 `Unavailable`, not a denial |
| Revocation of a delegation that has expired | Accepted, and evictable |

## 5. Open questions

1. ~~Concurrent writes.~~ Settled: key-per-pair, so there is no shared value to lose. See §2.2.
2. **Is the `path` witness required or optional?** The spec makes it MAY, and names exactly the reason to require it:

   > issuing spurious Revocations and requiring them to be stored is a potential DoS vector. Executors MAY require a delegation path witness be included to avoid this situation.

   Without it, anyone can make us store entries against any subject. Leaning required.
3. **Backfill.** Existing revocations live in R2. A one-time migration into KV, or accept that pre-existing revocations lapse? The registry is small today, so migration is cheap.
4. **Eviction.** The spec permits dropping a revocation once its target expires plus clock skew. `VerifiedRevocation` already captures `target_expires_at`, so the data is there. Defer until sets are big enough to matter.

   Note the spec also says revocation MUST NOT be reversible and there is no temporary hold: X.509 has one, UCAN deliberately does not, and the suggested equivalent is revoking and reissuing with a future `nbf`.
5. **Chain-depth cap.** `CHAIN_DEPTH_MAX` already exists in the metering plan and bounds CPU per request. With a single per-subject read the revocation check adds no per-CID cost, so the cap only needs to keep covering chain verification itself.
6. **Fail-open or fail-closed on a store outage.** Keep "revoked" and "could not check" distinct: a KV read error is `Unavailable` (503, the service's own fault, unbilled) rather than a denial. Storacha conflates the two and has an acknowledged TODO about it, which is worth not copying.
7. **Eviction.** The spec permits dropping a revocation once its target expires, plus clock skew. Worth doing to bound the per-subject set, but it needs the target's `exp`, which means either storing it alongside or re-reading the delegation. Defer until the set is big enough to matter.

## 6. Sequencing

This unblocks the rest:

```
revocations at /ucan/  →  decommission the account service  →  /customer/purge
```

The account service's remaining surface after this lands is `/devices/*`, `/account/summary`, `/accounts`, `/accounts/preflight`, `/codes`, and `/links/*`. Device rows become facts on the account space (the CLI already reads spaces that way); email availability and verification move to the access service, which already owns enroll and activate; `/links/*` is deleted, since `tonk account link` uses the loopback-callback flow and never touches it.
