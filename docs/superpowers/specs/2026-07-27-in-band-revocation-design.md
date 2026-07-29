# In-band revocation

Design for making revocation enforcement spec-shaped: every revocation
names a delegation CID that appears in the chains it kills, the access
service enforces a replicated monotone set instead of querying D1 per
presign, and issuer-DID matching is deleted. Written 2026-07-27 and
revised 2026-07-28 after settling the passkey-before-space identity
model. It follows the semantics of the UCAN revocation specification
while retaining Tonk's dialog-UCAN encoding.

## Problem

Revocation today is out-of-band with respect to the chains being
verified. A space created before account attachment delegates directly
to a device. Attaching an account later adds a disjoint
`root → device` grant without replacing that original route. Revoking
the grant by CID therefore catches nothing on the direct route. This is
why the access-service screen also matches issuer DIDs — a broadening
beyond the spec that forces a per-DID D1 lookup on every presign, makes
D1 the source of truth for a security property, and leaves the screen
untestable natively because it is welded to a wasm-only D1 binding.

The historical ordering causes the problem: a device receives space
authority first and a root identity may appear later. The new ordering
is root first. A user creates a local passkey root before creating a
space or becoming a durable member. Creating an account later attaches
provider services to that root; it does not change any space chain.

The spec's model is simpler: a revocation is a negative credential
naming a delegation CID, signed by an issuer in that delegation's proof
chain or by a principal delegated that authority, and enforced by the
executor. Denial requires no authority — an executor may always refuse.
For each Tonk remote, that executor is the presign boundary. What the
current chains lack is the delegations worth revoking.

## Goals

- The `root → device` grant appears in every durable, identity-bearing
  remote chain, so revoking it by CID severs everything that device can
  present through the root.
- Passkey-root creation remains local. A user needs no Tonk account or
  account-service round trip to create a space.
- Account attachment never re-roots a space. It associates email,
  recovery, backup, and discovery services with an existing root.
- Opening an open invite remains passkey-free. The link is deliberately
  a reusable bearer capability whose holder may read, write, and
  redelegate.
- A targeted invite addresses a root DID, not an account record.
- The access service enforces a locally replicated, verifiable, monotone
  set of revoked CIDs. No D1 on the hot path; D1 demotes to a
  best-effort UI index.
- The authority chain does not name Tonk's account service. Competing
  account and access providers can verify the same signed artifacts.

## Non-goals

- Making open invites read-only or single-use. Permission attenuation
  and more precise invitation policies can be added later.
- Cryptographically proving that a root DID came from a passkey. A
  passkey-derived root is an ordinary `did:key`; the creation and join
  clients enforce the ceremony, not the UCAN verifier.
- Root rotation and passkey-loss recovery.
- Discovery and request UX for targeted invitations.
- Upstream dialog changes. The session-rotation workaround for
  `prove`'s missing time bound stays as-is; fixing it upstream is a
  separate effort.
- Migrating spaces created before this ships (see Migration).

## Design

### Chain shapes

Terminology: the device key is the worker profile key; the session is
the bounded `device → operator` delegation `session.rs` already mints.
An "unregistered" user has a passkey root but has not attached an email
account or another provider.

| Situation | Chain |
|---|---|
| created, with or without an account | `space → root → device → session` |
| open visitor | `space → inviter → invite-key [→ guest session]` |
| durable join from an open invite | `space → inviter → invite-key → recipient root → device → session` |
| targeted invite | `space → inviter → recipient root → device → session` |

`inviter` abbreviates the inviter's valid authority path. Account
attachment does not add a hop or change any CID. Every durable member
route contains that member's `root → device` grant. The open visitor is
the intentional exception: the link itself is independent bearer
authority, not a device identity.

### Identity before account

The passkey root is identity and authority. An account is an optional
provider relationship around it.

Before creating a first space or durably joining one, the client:

1. Creates or evaluates a discoverable passkey locally.
2. Derives the root signer from its PRF output.
3. Mints the subject-open, command-open, unexpiring
   `root → device` delegation.
4. Stores that delegation with the device profile.

No account service participates. The root signer exists only for the
user-mediated passkey ceremony; routine work continues through the
device and bounded session keys.

The WebAuthn user handle and credential label must not depend on an
email address or account-provider identifier. Registration uses an
opaque local handle and a provider-neutral label. Attaching an account
later records its metadata beside the root; it does not rename or
replace the passkey.

Creating an account later proves control of the existing root and adds
email, recovery, device inventory, backup, or discovery services. It
does not mint a new root, replace the `root → device` delegation, or
rewrite any space chain. D1 may index the association and project
device status for UI, but neither field grants nor withdraws authority.

### Space creation

Space creation requires an available passkey root. The ephemeral space
signer delegates directly to that root and is then discarded:

1. Ensure the local `root → device` delegation exists.
2. Generate the space signer.
3. Mint `space → root`, scoped to the space.
4. Compose and store `space → root → device`.
5. Destroy the space signer.

There is no direct `space → device` grant, anchor, pre-signed
revocation, enrollment bundle, or account-service round trip. The same
ceremony runs whether or not the root is registered with any provider.

### Open invites

An open invite is a reusable bearer capability. Its URL fragment
contains the invite signer's seed, and its capability is currently
command-open. Anyone holding it may read, write, and redelegate. This
is accepted behavior, not a read-only security boundary.

Opening the link does not require a passkey. The visitor can use the
invite key directly or through a disposable guest session to load and
change the space. Until they join, the client need not create durable
membership, add a roster identity, or persist the invite into a root's
account backup.

"Join" means making that access durable:

1. Create or evaluate the recipient's passkey root.
2. Have the invite key delegate to the recipient root.
3. Compose the recipient's existing `root → device` delegation.
4. Persist the resulting chain and durable membership.

This passkey requirement is product policy, not a property an executor
can infer from the chain. Because the invite seed is already in the
holder's hands, another client can delegate to an ordinary key and
continue without a passkey. That is within the authority conveyed by an
open invite.

The same fact determines revocation granularity. Revoking a descendant
grant does not neutralize a holder who still has the invite seed; they
can mint another descendant. Closing an open invite means revoking the
delegation to the invite key. That also invalidates every durable member
whose only chain descends from that link. Future targeted grants may
promote members off the bearer branch before it is closed. Read-only,
single-use, and attenuated open links are deferred.

### Targeted invites

A targeted invite names the recipient root DID directly and carries no
bearer seed. The recipient must already have a passkey root, but need
not have a Tonk account. An account provider may help discover a root
DID; that lookup is convenience, not authorization, and the resulting
UCAN names the root rather than an account-service record.

Because only the targeted root can extend the chain through its device
grant, the recipient cannot retarget the invitation. Revoking that
targeted delegation removes that member without affecting unrelated
members or open links.

### Device revocation

The #646 ceremony is unchanged in authority (a device revokes itself;
only the root revokes another device) and target
(`mint_root_revocation` and `mint_self_revocation` already name the
grant's delegation CID). Publication adds a path witness where one is
not already present. The artifact enters the monotone set, and because
the grant CID is now in every durable chain for that device, CID
matching alone enforces it. The `root → device` grant remains
unexpiring and stable, so there is exactly one CID to revoke and no
overlapping renewal grant that could bypass the revocation. The D1
status flip follows the verified artifact as UI state; it is not
enforcement.

An open invite held by the same person remains an independent route.
Using it after device revocation is not a bypass around the revoked
grant: it is exercise of the separate bearer authority intentionally
conveyed by the link. Revoke the invite delegation to remove that route.

### UCAN compatibility boundary

The normative semantics come from
[`ucan-wg/revocation`](https://github.com/ucan-wg/revocation):

- a revocation irreversibly names a delegation by canonical CID;
- an issuer in the delegation's proof path may revoke it;
- revocation authority may itself be delegated;
- a path witness may accompany the revocation so a recipient can verify
  the revoker's position without prior local state; and
- executors check every presented delegation CID against a monotone
  revocation cache.

Tonk's dialog-UCAN form uses command `["ucan", "revoke"]` and argument
`revoke`, matching the working-group action and argument model.
Storacha's ucanto implementation expresses the same operation in its
newer capability vocabulary using `can`, `with`, `nb.ucan`, and
`nb.proof`. These representations are not byte-compatible. Tonk should
keep the semantic mapping explicit and must not claim ucanto wire
compatibility.

Every artifact accepted into the provider-independent set must carry
enough of the target chain to verify the revoker's authority. The
current device-signed self-revocation already carries its
`root → device` proof. Root-signed and invite revocations must add the
equivalent path witness instead of relying on a D1 device row.

### The monotone revocation set

A flat, append-only set of revoked delegation CIDs, each entry backed by
a verified artifact in the `revocations/` R2 namespace. By-CID entries
are globally unambiguous, so the hot-path representation is one global
set per executor — no account resolution.

- **Object shape**: each artifact is stored at an immutable,
  content-addressed key such as
  `revocations/<target-delegation-cid>/<artifact-cid>`. Re-publishing
  identical bytes is idempotent. Registry credentials cannot delete or
  overwrite existing entries.
- **Write path**: a relay verifies the artifact and its path witness,
  writes the immutable R2 object, then updates D1 as a best-effort UI
  projection. R2 succeeds first. A D1 failure can make the UI stale but
  cannot make the revocation absent from enforcement.
- **Read path**: the access service paginates the complete
  `revocations/` prefix on the existing cadence
  (`REVOCATION_TTL_MS`, 60 s), verifies unseen artifacts, and unions
  their target CIDs into its in-memory set. A refresh becomes
  authoritative only after every page succeeds.
- **No mutable index**:
  [R2 object listing is strongly consistent](https://developers.cloudflare.com/r2/reference/consistency/)
  through the Worker binding, so immutable per-artifact keys avoid both
  last-writer-wins index races and dependence on a D1 snapshot. A
  storage provider without strongly consistent listing must supply an
  append-only feed with detectable gaps instead.
- **Fail-closed**: unchanged semantics from #644/#646 — a fresh set is
  authoritative for 60 s, usable for a further 10 minutes only to ride
  out an unreachable store, refused beyond that.
- **Trust**: membership is self-certifying. Any consumer can fetch an
  artifact and re-verify its signature, target CID, and path witness.
  Completeness is a delivery property, not something signatures prove:
  an executor trusts its configured relay or mirrors not to omit an
  artifact. The monotone format lets executors union multiple relays
  without coordination.
- **Eviction**: an entry is evictable once the delegation it names would
  have expired anyway. `root → device`, open-invite, and targeted-member
  grants are currently unexpiring, so their revocations are retained
  indefinitely. Growth is proportional to revoked grants, not active
  users or invocations. This is accepted for the first implementation.

### Access service changes

- Keep the window screen (`expiry.rs`) and `collect_presented`'s CID
  gathering.
- Replace `D1RevocationRegistry` and `assess`'s issuer-DID matching with
  membership tests of the presented `delegation_cids` (and the
  invocation's proof CIDs) against the replicated set.
- Delete the `ACCOUNTS_DB` D1 binding from the access service entirely.
- The screen becomes natively testable: the set is plain data injected
  in tests, no wasm-only binding in the way.

### Sessions and grant bounds

The bounded session hop remains unchanged. It limits a captured hot key
and keeps routine signing off the device key, but it does not carry
device revocation.

The `root → device` grant deliberately remains unexpiring. A stable
grant gives each device one revocation CID, requires no periodic passkey
prompt, and keeps an offline device usable until the user explicitly
revokes it. If grants become expiring later, renewal must not leave
overlapping live CIDs that a single device-revocation action fails to
withdraw.

### Provider portability and offline behavior

Space and membership chains contain DIDs and signed delegations, not
Tonk account identifiers. An alternative account provider can attach
services to the same root, and an alternative access provider can
verify the same chains and revocation artifacts. Providers need no
shared device-status database. A new access provider does need a
configured revocation relay or mirror because revocation delivery, like
remote sync, is an online concern.

This portability is structural, not automatic discovery. The remote
configuration must identify the executor and its revocation submission
or mirror endpoints. A portable locator for those endpoints remains
separate work; the signed artifact format means defining it does not
require Tonk to become the authority.

The passkey ceremony itself is local and does not contact the Tonk
account service. After the root delegates to a device, ordinary space
creation, editing, and invitation work can proceed locally; only remote
sync and remote revocation propagation require a provider.

There is still an origin dependency. Current web passkeys are pinned to
the `tonk.spot` relying-party boundary. That is intentionally narrower
than account-service trust, but another web origin cannot evaluate the
same passkey PRF. Related Origin Requests, a local key agent, or another
portable root ceremony may reduce that dependency later. This design
does not widen the RP boundary.

## Threat model

Guaranteed: a device created honestly and compromised later is fully
revocable on durable member routes. Revoking its `root → device` grant
kills every such chain it can present because the grant CID is present
in all of them. Everything the device minted downstream, including
sessions and further delegations, dies with that hop.

Bounded, not guaranteed:

- **Open bearer authority**: device revocation does not withdraw an open
  invite seed the same person also holds. The link is an independent
  authority route and must be revoked by its own delegation CID.
- **Passkey provenance**: the protocol proves control of a root key, not
  that its seed came from WebAuthn PRF output. A modified client can use
  an ordinary key where the product requires a passkey.
- **Malicious at creation**: malware present while the ephemeral space
  signer exists could mint another first-hop delegation. Key
  destruction is not provable, and no later device revocation can name
  an undisclosed branch.
- **Propagation**: one refresh interval (60 s) between publish and
  enforcement, plus the 10-minute stale grace only during an outage.
  Unchanged from the current design.
- **Delivery completeness**: signatures prove that an observed
  revocation is valid, not that a relay exposed every valid revocation.
  Executors rely on their configured relay or mirrors for completeness.
- **Local replicas**: revocation gates the remote, not data already
  replicated. A revoked device keeps what it has; it stops being able to
  reach, pull, or push. By design.
- **Legacy spaces** (created before this ships): one first hop, no
  passkey root, issuer gone. Their direct device route cannot be made
  revocable by adding a separate root chain. See Migration.

## Migration

None for existing data. The account system is days old and pre-release;
existing accounts, devices, and spaces are development artifacts and
will be recreated rather than migrated. New profiles create or evaluate
a passkey root before creating a space or durably joining one. The D1
`devices` table and its status flags remain for account UI and
reconciliation; the access service stops reading them.

## Testing

- **Root-first ceremonies** (tonk-identity / tonk-worker): creation is
  refused without a local root; the space delegates directly to the
  root; no `space → device` grant is minted; creating an account later
  leaves the chain and all delegation CIDs unchanged.
- **Open invite** (tonk-invite / tonk-worker): an anonymous holder can
  read and write without a passkey; durable join delegates through a
  passkey root; revoking only a descendant does not stop a holder from
  redelegating; revoking the invite hop kills every descendant route.
- **Targeted invite**: the audience is a root DID independent of account
  registration; another root cannot claim it; revoking the targeted hop
  leaves unrelated members working.
- **Revocation artifacts** (tonk-identity / tonk-account-service):
  root, self, and invite revocations name the exact delegation CID and
  carry a sufficient path witness; a decoy witness or unauthorized
  issuer is rejected. Style per repo convention:
  `#[dialog_common::test]`, `it_does_x`.
- **Set mechanics** (native): append-only, idempotent re-publish,
  complete pagination, union-only refresh, invalid artifacts rejected,
  no D1 dependency, and eviction only past the named delegation's
  expiry.
- **Screen** (tonk-access-service, native now): a chain containing a
  revoked CID is refused; a chain free of them passes with no registry
  contact; stale-set grace and fail-closed windows behave as today.
- **Offline/provider boundary**: an unregistered passkey root creates
  and edits a local space without the account service; attaching an
  account changes no authority chain; the same chain verifies against a
  second access-provider fixture.
- **End to end** (wasm/browser, existing harness): create a passkey
  without an account, create a space, attach an account, verify sync,
  revoke the device root-signed, and verify the durable route is refused
  within one refresh interval.

## Consequences

- Every created space and durable member begins with a passkey root.
  "Accountless" now means unregistered, not rootless.
- Account attachment becomes a provider association rather than an
  authority migration. Anchors, escrow chains, pre-signed revocations,
  and link-time re-rooting disappear.
- Open invites remain full anonymous bearer authority. A passkey makes
  membership durable; it is not an authorization gate for the link.
- Targeted invitations bind directly to root DIDs and do not require an
  account provider.
- The UCAN revocation invariant applies cleanly in the enforcement path,
  while Tonk's dialog encoding remains an explicit compatibility layer.
- The access service holds no keys, no authority, and no database — it
  verifies chains and consults a replica it can audit.
- D1 device status is a projection of signed revocation state. It is
  never consulted to authorize a request.
- Unexpiring grants make revocation entries permanent and the set grows
  with revoked grants. This buys one stable CID per device and avoids
  periodic passkey prompts or renewal bypasses.
- The core is independent of Tonk's hosted account service, but the
  current passkey ceremony still depends on the `tonk.spot` RP boundary.
