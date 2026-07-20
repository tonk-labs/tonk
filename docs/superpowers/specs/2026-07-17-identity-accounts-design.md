# Cross-device identity and accounts

Design for a user identity that spans devices, with account recovery and
payment rails layered on top. Approved 2026-07-17. Revised 2026-07-20: the
root key is now derived from a passkey via the WebAuthn PRF extension;
passphrase escrow is removed and tonk holds no key material.

## Problem

Tonk identity today is one Ed25519 keypair per device (`dialog_operator::Profile`,
IndexedDB in the browser, `dirs::data_dir()/dialog/tonk` on native). Each device
is an unrelated DID: there is no way to be one user on two devices, no recovery
if a device is lost, and nothing to hang billing on. The membership schema was
explicitly designed so multi-device claims converge on one roster row — but that
convergence assumes a shared DID the system does not yet provide.

## Decisions

- One design covering identity and payments; built in stages.
- Crypto-first: keys stay canonical. No server owns identity, and tonk holds
  no user key material — not even ciphertext.
- Root user DID with per-device DIDs linked by UCAN delegation (not a shared
  seed copied between devices).
- The root key is **derived from a passkey** (WebAuthn PRF extension), never
  stored. One passkey per account; platform credential sync (iCloud Keychain,
  Google Password Manager) is the cross-device transport and the redundancy
  story.
- No escrow. Recovery is passkey sync; rotation and passkey loss are handled
  by succession delegations already expressible in the model; total loss
  re-anchors the account to a new root DID.
- Accounts are located by verified email.
- Payments are fiat via Stripe end-to-end; user-to-user later via Stripe
  Connect. No crypto rails.
- Subscription gates sync/storage, enforced at the access-service.
- Server shape: a new `tonk-account-service` worker beside the gateway;
  `tonk-access-service` stays a capability verifier and gains one entitlement
  lookup.

## Identity model

The user is a keypair; devices are delegates. The keypair comes from the
user's passkey.

### Root key from the passkey

At account creation the user creates one passkey on the tonk origin. The root
Ed25519 seed is derived on demand:

```
PRF(credential, "tonk/root-key/v1") → HKDF-SHA256 → Ed25519 seed
```

- The PRF eval input is a fixed, versioned public constant — no per-account
  salt, no server round trip before derivation. The version bumps only if the
  derivation scheme must change, which is a deliberate rotation (below).
- The root key exists in memory only for the seconds a ceremony needs it, then
  is wiped. It is never written to disk, IndexedDB, or the server. At rest the
  root key does not exist; it is re-derived from the passkey each time.
- The root DID (`did:key` of the derived public key) is the user's identity
  everywhere. The device's existing profile keypair is untouched.
- Each derivation is one WebAuthn `get()` with a user gesture — a biometric
  prompt per ceremony, not per operation. Day-to-day operation never touches
  the root key; devices run on their delegations.
- Some platforms do not return PRF output from `create()`; the creation
  ceremony performs a follow-up `get()` to derive the root before registering
  the account.

Derived-from-passkey is chosen over random-key-plus-escrow because there is
nothing to store, nothing for tonk to hold, and no passphrase to forget. The
trade-off is that the identity is welded to one credential: no second passkey
ecosystem, and losing the passkey with no linked device is unrecoverable key
loss (see Recovery). Platform sync makes that failure rare; the design accepts
it in exchange for zero custody.

Passkeys are scoped to the tonk origin, so derivation happens only in a
browser context on that origin. Native and CLI devices participate through a
browser handoff (below).

### Devices are delegates

- The root key mints a UCAN delegation `root → device`. Devices operate
  purely on their delegation.
- The `root → device` delegation is **subject-open, audience-specific**
  (`Subject::Any`, audience = device DID): "this device may act as me, for
  anything". This is deliberately the opposite shape from space invites, which
  are subject-specific and must remain so. It is the same powerline pattern
  used when space keys delegate to admins and are discarded.
- Space access composes chains: a device presents
  `[space → … → root, root → device]`.

### Rosters key on the root DID

`Membership`, `MemberRole`, `MemberName`, `InvitedVia` use the root DID as the
member. The content-derived `(subject, member)` entity then makes claims from
any of the user's devices converge on one roster row. Sigils, petnames, and
display names hang off the root DID, so a user is one person in the UI
regardless of device.

Invite claims redelegate the ephemeral key to the **root DID** as audience. No
root key is needed at claim time: the claiming device signs the redelegation
with the ephemeral seed from the invite URL — delegating *to* a DID needs no
signature from the audience — and its own `root → device` link completes the
chain. A linked device knows its root DID without holding the root key: the
`root → device` delegation names the root as issuer. A device without an
account claims with its device DID exactly as today; the two coexist.

### Device linking

The passkey is the transport, so linking is self-service — no second device,
no QR ceremony for browsers:

1. New browser device signs in with the synced passkey (one biometric
   prompt).
2. Root key, derived in memory, mints `root → thisDevice`.
3. Device is registered with the account service; root key is wiped.

Native and CLI devices, which lack WebAuthn, link via browser handoff: the
CLI shows its device DID (QR or short code), a browser session on the tonk
origin derives the root and mints `root → cliDevice`, and the delegation is
handed back through the account service.

Direct root delegation is chosen over device chains (`A → B`) so revocation
stays atomic: one device, one delegation, and revoking a device cannot orphan
others.

### Re-anchoring pre-link claims

A device that claimed spaces before linking holds chains terminating at its
device DID (`space → eph → device`), which other devices cannot use. At link
time the device re-anchors: for each held capability it mints
`device → rootDID` (its own key suffices; no ceremony), saves it to the shared
UCAN store, and re-asserts its roster rows under the root DID. Those chains
become `space → eph → device → root → otherDevice` — longer but valid.

Trade-off: re-anchored chains flow through the old device DID, so revoking
that device later severs them. The clean-up is a fresh invite claim or a
founder re-delegation; rosters keyed on the root DID make the affected spaces
discoverable.

### Revocation

A registry concern, not a chain rewrite. The account service keeps the device
registry (device DID, delegation CID, display name, status). Revoking a device
marks its delegation CID revoked; the access-service checks the revocation list
when verifying chains. Data already on a stolen device is unprotectable
(local-first), but its sync access dies at the gateway.

### Migration for existing users

On account creation, the device re-asserts its memberships under the root DID
on each space it can write to and retracts the old device-DID rows. Care:
first-wins stamps (`MemberRole`, `InvitedVia`) must be re-stamped on the new
entity, not assumed to carry over. Users without accounts see no change;
nothing forces account creation.

## Recovery and rotation

There is no escrow. Recovery decomposes into cases, all built from the same
delegation machinery — no new cryptographic artifacts:

- **Normal case: passkey sync is key recovery.** A new or wiped device signs
  in with the synced passkey and self-links. Losing every device loses
  nothing as long as the platform credential survives.
- **Deliberate rotation** (new passkey, scheme version bump, suspected
  compromise with the passkey still in hand): derive the old root, mint a
  subject-open succession delegation `oldRoot → newRoot`, re-assert rosters
  under the new DID. Old chains stay valid through the succession link
  (`space → … → oldRoot → newRoot`); fresh invites shorten them over time.
- **Passkey lost, a linked device survives:** devices hold subject-open
  `root → device` delegations, and UCANs re-delegate. The surviving device
  mints `device → newRoot` for a freshly created passkey, giving the new root
  a valid chain into everything the account had. The account row flips to the
  new root DID and the old credential is revoked. Same trade-off as
  re-anchoring: those chains flow through that device DID until fresh invites
  replace them.
- **Total loss (no passkey, no device):** unrecoverable key loss, by design —
  the price of zero custody. Email verification plus support contact
  **re-anchors the account**: the account row points to a newly derived root
  DID, billing and entitlements carry over, but space access does not.
  Rosters keyed on the old root DID make affected spaces discoverable, and
  founders re-invite. Account recovery is not key recovery, and the spec — and
  the creation UX — say so loudly.

The device's persisted space-delegation chains (the UCAN store) are backed up
server-side so a recovered account lands with its spaces. They are capability
tokens the gateway already sees on every presign, not secrets of root weight.

### Platforms without PRF

Browsers or authenticators without the PRF extension cannot create accounts;
those users stay device-only, exactly as today. Nothing forces account
creation, so this degrades cleanly and shrinks as platform support grows.

## `tonk-account-service`

New Cloudflare Worker (workers-rs, same stack as the access-service) with D1.

Tables:

| table | contents |
|---|---|
| `accounts` | id, verified email, root DID, passkey credential id, created |
| `devices` | account id, device DID, delegation CID, display name, status |
| `chains` | account id, backed-up space-delegation chains (UCAN store) |
| `entitlements` | root DID, plan, status, limits, Stripe customer/subscription ids |

Authentication reuses the gateway's pattern: UCAN invocations signed by the
device key with the `root → device` chain attached. No sessions, no passwords.
Two ceremonies where no delegation exists yet use email codes instead:

- **Account creation**: email + verification code binds email → root DID and
  registers the passkey credential id and first device.
- **Re-anchor**: email code plus support contact authorizes pointing the
  account row at a new root DID (total-loss path only; rate-limited, logged).

### Failure posture

Account-service downtime never blocks local work or space sync — devices hold
their delegations, and root derivation needs no server at all. Only ceremonies
(link, re-anchor, billing changes) and revocation-list freshness degrade. The
gateway caches the revocation list with a short TTL and fails open on
account-service outage.

## Billing and entitlements

Stripe, boring on purpose: Checkout Session to subscribe, Customer Portal to
manage and cancel; no card UI of our own. The account service holds the
root DID → Stripe customer mapping, receives webhooks (idempotent,
signature-verified, replayable), and reduces them into `entitlements`: plan,
status, concrete limits (space count, storage bytes, possibly bandwidth). The
account email doubles as the Stripe receipt email.

Enforcement is **requester-based at the gateway**. On each presign,
`tonk-access-service` resolves DIDs from the chain it already parses against a
D1 view of `devices` + `entitlements` + the revocation list — one indexed
lookup. Unknown DIDs get the free tier. Lapsed accounts get free-tier limits,
not lockout: local-first software degrades to "your extra spaces stop
syncing", never "your data is hostage".

Requester-based is chosen over space-owner-based because the gateway can see
the requester in the chain it verifies, while space ownership lives in repo
content it cannot read. Per-space owner billing can layer on later by having
the account service track owned subjects at space creation.

### User-to-user payments (intent only)

Stripe Connect Express accounts attached to the same account row. The identity
layer's contribution is the stable root DID → account mapping and a verified
email. Nothing in the current build special-cases it; deliberately deferred.

## Build order

Four PR-sized stages, each independently useful:

1. **Account service skeleton** — worker + D1, email verification, device
   registry CRUD, chain backup put/get. No client changes, no Stripe, no
   gateway changes.
2. **Client ceremonies** — account creation (passkey create, PRF derivation,
   `root → device`), self-link on a new browser, CLI browser handoff.
   Profile machinery untouched; new code sits beside the two provisioning
   call sites (`rust/tonk-cli/src/identity.rs`,
   `rust/tonk-worker/src/worker.rs`).
3. **Root-DID rosters** — invite claims audience the root DID, membership
   facts keyed by root DID, migration re-assertion for existing members.
4. **Billing** — Stripe integration, entitlements, gateway lookup (shipped
   fail-open/log-only first, then enforced).

Succession and surviving-device recovery ceremonies ride with stage 2 or
immediately after; they reuse its delegation plumbing.

## Testing

- Root derivation (PRF output → HKDF → seed) and chain construction as
  dialog-style unit tests with fixed vectors.
- Account-service handlers against local D1.
- Passkey ceremonies end-to-end against a CDP virtual authenticator with
  `hmac-secret`/PRF enabled — creation, self-link, surviving-device recovery
  as bench scenarios.
- Stripe via test clocks and replayed webhook payloads.
- The gateway entitlement check soaks in log-only mode before enforcement.

## Risks

- **Total loss is identity loss.** No passkey and no device means the root
  key is gone; only the account re-anchors. Mitigate with loud UX at creation
  ("your passkey is your identity") and by making self-link frictionless so
  users have more than one linked device.
- **Platform lock-in.** The credential lives in one vendor's sync fabric;
  leaving the ecosystem without a linked device is the total-loss path.
  Surviving-device recovery is the practical exit ramp.
- **PRF availability gaps** keep some users device-only. Acceptable: nothing
  forces accounts, and support is broad and growing.
- **PRF-at-create quirks** — platforms that need a follow-up `get()` make the
  creation ceremony two prompts. Cosmetic, but the ceremony must handle both
  shapes.
- **Fail-open revocation** leaves a revoked device a brief sync window during
  an account-service outage. Accepted in exchange for sync availability.
- **Migration re-assertion** must not trip first-wins stamps (`MemberRole`,
  `InvitedVia`) — the new root-keyed entities need explicit re-stamping.
