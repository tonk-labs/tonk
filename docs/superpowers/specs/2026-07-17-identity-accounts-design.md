# Cross-device identity and accounts

Design for a user identity that spans devices, with account recovery and
payment rails layered on top. Approved 2026-07-17.

## Problem

Tonk identity today is one Ed25519 keypair per device (`dialog_operator::Profile`,
IndexedDB in the browser, `dirs::data_dir()/dialog/tonk` on native). Each device
is an unrelated DID: there is no way to be one user on two devices, no recovery
if a device is lost, and nothing to hang billing on. The membership schema was
explicitly designed so multi-device claims converge on one roster row — but that
convergence assumes a shared DID the system does not yet provide.

## Decisions

- One design covering identity and payments; built in stages.
- Crypto-first: keys stay canonical. No server owns identity.
- Root user DID with per-device DIDs linked by UCAN delegation (not a shared
  seed copied between devices).
- Root key recovery via client-side-encrypted escrow held by us.
- Accounts are located by verified email.
- Payments are fiat via Stripe end-to-end; user-to-user later via Stripe
  Connect. No crypto rails.
- Subscription gates sync/storage, enforced at the access-service.
- Server shape: a new `tonk-account-service` worker beside the gateway;
  `tonk-access-service` stays a capability verifier and gains one entitlement
  lookup.

## Identity model

The user is a keypair; devices are delegates.

- At account creation the first device generates a second Ed25519 keypair, the
  **root user key**. Its `did:key` is the user's identity everywhere. The
  device's existing profile keypair is untouched.
- The root key mints a UCAN delegation `root → device` and then leaves the
  device. At rest it exists only as the encrypted escrow blob. Devices operate
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

A ceremony on an existing device:

1. New device shows its DID (QR or short code).
2. Existing device fetches the escrow blob and decrypts it (passphrase entry).
3. Root key, in memory only, mints `root → newDevice`.
4. New device is registered with the account service; root key is wiped.

Direct root delegation is chosen over device chains (`A → B`) so revocation
stays atomic: one device, one delegation, and revoking a device cannot orphan
others.

A device with no other linked device self-links via the recovery-shaped flow:
email + code fetches the escrow blob, the passphrase decrypts it locally on
that device, and the root key in memory mints `root → thisProfile` before
being wiped. A second device is never required, because custody of the
ciphertext is server-side.

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

## Escrow and recovery

### Escrow blob

Root key seed encrypted client-side: passphrase → Argon2id → XChaCha20-
Poly1305, in a versioned envelope (KDF parameters inside) so schemes can
rotate. The server stores ciphertext it cannot read. Passkey-PRF unlock can be
added later as a second envelope recipient; designed-for, not built now.

The device's persisted space-delegation chains (the UCAN store) are backed up
alongside the escrow blob. They are capability tokens, not secrets of the same
weight, and without them recovery lands in an account with no spaces.

### `tonk-account-service`

New Cloudflare Worker (workers-rs, same stack as the access-service) with D1.

Tables:

| table | contents |
|---|---|
| `accounts` | id, verified email, root DID, created |
| `escrow` | account id, envelope blob, version |
| `devices` | account id, device DID, delegation CID, display name, status |
| `entitlements` | root DID, plan, status, limits, Stripe customer/subscription ids |

Authentication reuses the gateway's pattern: UCAN invocations signed by the
device key with the `root → device` chain attached. No sessions, no passwords.
Two ceremonies where no delegation exists yet use email codes instead:

- **Account creation**: email + verification code binds email → root DID,
  uploads escrow, registers the first device.
- **Recovery**: email code authorizes fetching the escrow blob only.
  Decryption still requires the passphrase, so a compromised inbox alone
  yields ciphertext.

### Recovery flow (all devices lost)

Fresh device: enter email → code → fetch escrow → decrypt locally with
passphrase → root key in memory → mint `root → thisDevice`, register it, wipe.
Spaces return via the backed-up delegation chains plus roster facts keyed to
the root DID. Email codes are rate-limited and escrow fetches logged. Panic
path is root rotation (new root, re-delegated while the old root is in memory
during a ceremony); the full rotation ceremony is designed but can ship later.

### Failure posture

Account-service downtime never blocks local work or space sync — devices hold
their delegations. Only ceremonies (link, recover, billing changes) and
revocation-list freshness degrade. The gateway caches the revocation list with
a short TTL and fails open on account-service outage.

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

1. **Account service skeleton** — worker + D1, email verification, escrow
   put/get, device registry CRUD. No client changes, no Stripe, no gateway
   changes.
2. **Client ceremonies** — account creation (root keygen, escrow upload,
   `root → device`), device linking, recovery. Profile machinery untouched;
   new code sits beside the two provisioning call sites
   (`rust/tonk-cli/src/identity.rs`, `rust/tonk-worker/src/worker.rs`).
3. **Root-DID rosters** — invite claims audience the root DID, membership
   facts keyed by root DID, migration re-assertion for existing members.
4. **Billing** — Stripe integration, entitlements, gateway lookup (shipped
   fail-open/log-only first, then enforced).

## Testing

- Chain construction and envelope encrypt/decrypt as dialog-style unit tests.
- Account-service handlers against local D1.
- Linking and recovery ceremonies as an end-to-end two-device scenario in the
  bench harness.
- Stripe via test clocks and replayed webhook payloads.
- The gateway entitlement check soaks in log-only mode before enforcement.

## Risks

- **Forgotten passphrases** make escrow decoration. Mitigate with loud UX at
  creation and passkey-PRF later.
- **Fail-open revocation** leaves a revoked device a brief sync window during
  an account-service outage. Accepted in exchange for sync availability.
- **Migration re-assertion** must not trip first-wins stamps (`MemberRole`,
  `InvitedVia`) — the new root-keyed entities need explicit re-stamping.
