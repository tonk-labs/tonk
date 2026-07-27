# In-band revocation

Design for making revocation enforcement spec-shaped: every revocation
names a delegation CID that appears in the chains it kills, the access
service enforces a replicated monotone set instead of querying D1 per
presign, and issuer-DID matching is deleted. Written 2026-07-27,
following the session-delegation and signed-revocation work in #646.
Guided by the UCAN revocation spec (github.com/ucan-wg/revocation).

## Problem

Revocation today is out-of-band with respect to the chains being
verified. The presign chain is `space → profile → operator`; the account
grant `root → device` is a disjoint chain that never appears in it. So
revoking the grant by CID would catch nothing, which is why the access
service screen matches on issuer DIDs as well as CIDs — a broadening
beyond the spec that forces a per-DID D1 lookup on every presign, makes
D1 the source of truth for a security property, and leaves the screen
untestable natively (it is welded to a wasm-only D1 binding).

The spec's model is simpler: a revocation is a negative credential naming
a delegation CID, signed by an issuer in that delegation's proof chain,
enforced by the executor (the party performing invocations). Denial
requires no authority — an executor may always refuse. Tonk already has
exactly one executor, the presign boundary. What it lacks is chain shapes
in which the delegations worth revoking actually appear.

## Goals

- The `root → device` grant appears in every remote-facing chain, so
  revoking it by CID severs everything the device can present.
- Direct grants that predate an account can be killed in-spec at link
  time, so a revoked device has no short chain to fall back to.
- The access service enforces a locally replicated, verifiable, monotone
  set of revoked CIDs. No D1 on the hot path; D1 demotes to a write log
  and UI index.
- Accountless creation, join, and sync keep working unchanged. Linking
  an account is what upgrades a space's chains, never a prerequisite.

## Non-goals

- Root rotation and passkey-loss recovery (existing restore ceremony is
  unchanged and out of scope).
- Upstream dialog changes. The session-rotation workaround for
  `prove`'s missing time bound stays as-is; fixing it upstream is a
  separate effort.
- Migrating spaces created before this ships (see Migration).

## Design

### Chain shapes

Terminology: the device key is the worker profile key; the session is
the bounded `device → operator` delegation `session.rs` already mints.

| Situation | Chain |
|---|---|
| created while linked | `space → root → device → session` |
| created accountless, then linked | `space → anchor → root → device → session` |
| joined while linked | `space → inviter → invite-key → root → device → session` |
| joined accountless, then linked | `space → inviter → invite-key → anchor → root → device → session` |
| accountless (never linked) | `space [→ inviter → invite-key] → device → session` |

The account root is a real hop in every linked chain. Revoking the
`root → device` grant CID therefore kills every route a linked device
has — no issuer matching needed.

### The anchor: re-rooting without retaining the space key

An accountless space's first hop is issued by a key that no longer
exists (the ephemeral space key, or an invite key), so its direct grant
to the device looks unrevocable. The fix is two extra signatures at the
one moment that issuer is alive, neither of which retains any key:

At **space creation** (accountless), the ephemeral space key K signs
four artifacts and is then destroyed exactly as today:

1. `space → device` — the working grant W. Daily life is unchanged.
2. `space → anchor` — a dormant branch to a fresh key the device holds
   sealed and never uses. The anchor is an ordinary delegate in the
   same trust class as the device key beside it; it cannot mint first
   hops or impersonate the space.
3. a revocation of W — inert bytes. A pre-signed revocation is an
   artifact, not authority: it can only withdraw, and its validity does
   not depend on when it was signed.
4. a revocation of the anchor hop — the break-glass artifact (below).

At **invite claim** (accountless), the invite key embedded in the link
plays K's role: it signs the working hop to the device, a dormant hop to
a fresh anchor, and pre-signed revocations of both, then is discarded.
No change to the invite format; the invite key is already an ephemeral
issuer the claimer briefly controls. The invite-subject invariant
(specific repo subject, never `Subject::Any`) is untouched.

When the ceremony runs **while linked**, the anchor is unnecessary: K
(or the invite key) delegates straight to the account root, signs the
break-glass revocation of that hop, and dies. No working grant to the
device ever exists.

### Space enrollment: atomic, unfakeable, visible

Linking a space into an account is a single bundle submitted to the
account service:

- the escrow chain `space → anchor → root` (anchor-signed; for joins,
  `space → inviter → invite-key → anchor → root`),
- the pre-signed revocation of the working grant W,
- the pre-signed break-glass revocation of the anchor hop.

The service verifies coherence before accepting anything: the escrow
chain verifies cryptographically, and each revocation's issuer must be
an issuer of a hop in that chain (the space DID for created spaces, the
invite key for joins). Then, in order: store the escrow chain, publish
the W revocation into the monotone set, store the break-glass artifact
unpublished. The device destroys the anchor after the service accepts.

Atomicity is the enforcement. The device cannot fake the artifacts —
only the dead issuer could have signed them, it signed exactly one
revocation per hop, and a revocation lifted from another space fails the
issuer-membership check. The device's only alternative is withholding
the space from the account entirely, which is visible: the space is not
backed up, not restorable, not listed. Protection claims and
revocability cannot come apart. Service-side ordering also removes the
strand-yourself hazard: the root route is verified before the working
grant's revocation goes live.

The old escrow path (`try_back_up_owned_space`, which mints
`space → device → root` through W) is replaced by this bundle; a chain
routed through W would die with W.

### Device revocation

The #646 ceremony is unchanged in authority (a device revokes itself;
only the root revokes another device) and in artifact shape (the
revocation names the grant's delegation CID — `mint_root_revocation` and
`mint_self_revocation` already do this). What changes is consequence:
the artifact is published into the monotone set, and because the grant
CID is now in every linked chain, CID matching alone enforces it. The
D1 status flip remains as UI state, not enforcement.

### Break-glass

Publishing the escrowed anchor-hop (or root-hop) revocation severs every
remote route to the space, the root's included. It is a deliberate nuke
for link-time compromise — "this space's linkage is untrustworthy,
migrate the data" — exposed as a root-authorized account-service action,
never triggered automatically. It exists because no ceremony can prove a
key was destroyed: a device malicious *at* link time could retain the
anchor and mint from it later. That case is outside the threat model
every key-destruction scheme already assumes (today's design trusts the
device to discard the space key at creation), but the artifact costs one
signature at a moment the issuer is already signing, and it gives the
unprovable case a recovery story.

### The monotone revocation set

A flat, append-only set of revoked delegation CIDs, each entry backed by
a verified artifact in the existing `revocations/` R2 namespace. By-CID
entries are globally unambiguous, so the hot-path representation is one
global set — no account resolution.

- **Write path**: the account service verifies an artifact (device
  revocation or enrollment bundle), appends it to R2, records it in the
  D1 write log, and rewrites a single global index object
  (`revocations/index`) from the log. D1 serializes writers; publishes
  are rare.
- **Read path**: the access service fetches the index object on the
  existing cadence (`REVOCATION_TTL_MS`, 60 s) via a read-only R2
  binding and holds the set in memory. Effect latency is one refresh
  interval, the same 60 s the current design accepts.
- **Fail-closed**: unchanged semantics from #644/#646 — a fresh set is
  authoritative for 60 s, usable for a further 10 minutes only to ride
  out an unreachable store, refused beyond that.
- **Trust**: the set is self-certifying. Any consumer can fetch the
  backing artifacts and re-verify signatures and issuer membership; the
  access service trusts its replica, not the account service's honesty.
- **Eviction**: an entry is evictable once the delegation it names would
  have expired anyway (the spec's expiry-preference). Session hops
  expire in hours; `root → device` grants become bounded (below) and
  evict on renewal cycles. First-hop revocations (W and break-glass)
  never evict — those grants are unexpiring — and grow by one entry per
  accountless enrollment, a rare one-time event. Accepted; if it ever
  matters, the fix is bounding first hops, not the enforcement path.

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

The session hop stays self-minted and keeps three jobs, none of them
revocation: a disposable hot key on the busiest signing path, bounded
chains so the revocation set can evict, and a replay bound on captured
containers. Renewal is local and free, so `SESSION_TTL_SECONDS` drops
from 12 h to 1 h, with `RENEWAL_MARGIN_SECONDS` at 15 minutes so the
margin stays inside the TTL. Rotation-on-renewal stays until the
upstream `prove` time-bound lands.

The `root → device` grant gains an expiry (30 days, tunable), renewed by
a passkey prompt. This is what lets device revocations evict from the
set, and it caps how long a never-revoked lost device stays capable. A
device offline past the bound re-links with its passkey and loses
nothing.

## Threat model

Guaranteed: a device honest at link time and compromised later is fully
revocable. Revoking its `root → device` grant kills every chain it can
present — root routes contain the grant CID, and its pre-account direct
grant was revoked at enrollment. Everything the device ever minted
downstream (sessions, further delegations) dies with the hops above it.

Bounded, not guaranteed:

- **Malicious at creation or link**: key destruction is unprovable.
  Malware present at creation could mint `space → attacker` directly —
  uncatchable today too (the space DID is nobody's revoked device, so
  even issuer matching misses it). The break-glass artifact is the
  recovery, not prevention.
- **Propagation**: one refresh interval (60 s) between publish and
  enforcement, plus the 10-minute stale grace only during an outage.
  Unchanged from the current design.
- **Local replicas**: revocation gates the remote, not data already
  replicated. A revoked device keeps what it has; it stops being able to
  reach, pull, or push. By design.
- **Legacy spaces** (created before this ships): one first hop, no
  artifacts, issuer gone. They re-root the lossy way — root hop added,
  old direct grant unrevocable. See Migration.

## Migration

None for existing data. The account system is days old and pre-release;
existing accounts and linked spaces are development artifacts and will be
recreated rather than migrated. The ceremonies above apply to spaces
created or joined from this point. The D1 `devices` table and its status
flags remain for the account UI; the access service simply stops reading
them.

## Testing

- **Ceremony units** (tonk-identity / tonk-worker): creation mints all
  four artifacts and the working chain still presigns; claim does the
  same through an invite key; linked-path ceremonies mint no working
  grant; the anchor never signs anything but its enrollment hop.
- **Enrollment** (tonk-account-service, native): bundle coherence — a
  revocation whose issuer is not in the escrow chain is rejected; a
  decoy from another space is rejected; ordering stores escrow before
  publishing; break-glass is stored unpublished. Style per repo
  convention: `#[dialog_common::test]`, `it_does_x`.
- **Set mechanics** (native): append-only, idempotent re-publish,
  eviction only past the named delegation's expiry, index rebuild from
  the log.
- **Screen** (tonk-access-service, native now): a chain containing a
  revoked CID is refused; a chain free of them passes with no registry
  contact; stale-set grace and fail-closed windows behave as today.
- **End to end** (wasm/browser, existing harness): link an accountless
  space, verify presign works via the root route, revoke the device
  root-signed, verify both the root route and the old direct chain are
  refused within one refresh interval.

## Consequences

- The spec's revocation model applies verbatim; nothing bespoke remains
  in the enforcement path.
- The access service holds no keys, no authority, and no database — it
  verifies chains and consults a replica it can audit.
- Sessions stop carrying the revocation burden and shrink to 1 h.
- One new sealed key (the anchor) and up to three extra signatures at
  ceremonies where the issuer already exists in memory. No new network
  round trips at creation or claim.
- Revocation UX is unchanged for users; what changes is that it now
  means what it says.
