# Enrollment, customer spaces, and atomic activation

## Two problems

**1. Signup can complete with an unusable account.** Custody is
provisioned at ceremony time, when the customer is at best `Registered`.
`provision_or_defer` (`rust/tonk-worker/src/router/customer.rs:479`)
queued only on `CustomerInactive`, so a call that raced ahead of the
asynchronous `tonk:enroll` command met `UnknownCustomer` and dropped its
consent. Only a live passkey assertion can mint that consent, so the
space stayed unprovisioned, the queued publish failed forever against
the presign gate (`provisioning.rs:76`), and no second device could sign
in. Fixed in step 1, but the underlying fault stands: activation and
custody are independent acts, so a client can fall out between them.

**2. The enrollment deposit is inert, and its check proves nothing
useful.** `verify_deposits` (`registration.rs:586`) compares the deposit
against scopes derived from `deposit_scopes`
(`rust/tonk-account/src/customer.rs:67`) — the same function the client
built from. It confirms the client followed instructions; it never asks
whether the grant covers what the service needs. Nothing has forced the
question, because nothing reads the deposit: it is written to
`customer.access` and no SELECT mentions that column. The service builds
no memory or archive invocation anywhere.

## Design

**The service owns the bookkeeping space.** Not for integrity — receipts
are service-signed and dialog keeps history, so a client can omit or
delete but never forge. The reason is revocation: a delegation the
client grants, the client can withdraw or under-scope, which would leave
the service's own bookkeeping at the customer's discretion. Owning the
space removes that dependency. The account gets `/use/get` back so it
can read its own record.

This deletes `deposit_scopes`, `service_space`, `SERVICE_CATALOG`,
`verify_deposits` and the `customer.access` column.

**Enrollment verifies; activation executes.** The recovery material
travels with enrollment and is checked there, so a malformed one fails
while the person is watching rather than silently later. Activation
performs everything in one transaction — no window to fall out of.

## Keys

The customer space's key is **derived**, not stored: HKDF over the
service seed with a versioned context and the account DID as info, the
same shape `custody_seed` uses (`tonk-identity/src/envelope.rs:73`).
Nothing is sealed, nothing is persisted, and the DID is recomputable
from the account DID at any time. Rotation is a context-version bump.

The endgame is a hardware key holding the service DID, with workers
holding only attenuated, revocable delegations from it. Derivation is
compatible: derived per-customer today, delegation-rooted once the
hardware key lands, and nothing depends on a stored space key that would
have to be migrated.

## Flow

### 1. Enroll

```
cmd: /customer/enroll
sub: did:key:zAlice          // account
args:
  email:    alice@web.mail
  recovery: <cid>    // /use/put/memory/cell, self-signed by zPsk
  consent:  <cid>    // zPsk -> zAlice, /consumer/provision or broader
  sealed:   <cid>    // the envelope the recovery invocation checksums
```

Every argument names a block in the same UCAN container — the container
is a DAG archive, as `access` already travels
(`deposited_delegation`, `registration.rs:569`). Arguments, not proofs.

The service checks, and writes only the customer row and the
reservations:

1. the email is not registered to a different account
2. `recovery` verifies — signature, subject is the passkey DID, its
   `checksum` matches `sealed`, and its expiry outlives the activation
   window
3. `consent` verifies — issued by the passkey DID, audienced to the
   account, command a prefix of `/consumer/provision`

It derives the customer space DID and reserves both that DID and the
custody DID.

### 2. Receipt

Service-signed, so it is a verifiable receipt.

```
cmd: /ucan/conclude
sub: did:web:network.tonk
args:
  inv: </customer/enroll>
  out:
    ok:
      status:   Registered
      provider: https://tonk.network/ucan/
      customer:
        cmd: /use/get
        sub: did:key:zcstr
        aud: did:key:zAlice
```

Read-only, unconstrained, on a subject holding nothing but this
account's bookkeeping. The narrowing is the identity, not the
capability.

### 3. Activation link

```
cmd: /customer/activate
sub: did:web:network.tonk
args:
  account:  did:key:zAlice
  customer: did:key:zcstr
  recovery: <cid>
  sealed:   <cid>
  consent:  <cid>
```

Service-signed, service-subjected, proofless (`registration.rs:176`), so
it needs no key on the presenting device and any browser finishes the
account.

**Carry only those three blocks.** Enrollment's container holds more —
the root->device link, and today `delegation_tokens`
(`registration.rs:546`) sweeps *every* delegation token into storage
regardless of what the arguments name. The link is the tightest budget
in this design; it must be built from the named blocks alone.

Already verified at enrollment; not re-verified here.

### 4. Activate — one transaction

1. write the recovery cell
2. convert the reservations: consumer rows for `zcstr`, `zAlice`, `zPsk`
3. set customer status `Active`

All or nothing.

## What enrollment must reject

Enrollment is the **only** verification point: activation deliberately
does not re-verify, so anything accepted here becomes a stranded account
later — the failure this design exists to prevent. Every refusal must
land before the customer row is written, so a rejected enrollment leaves
nothing behind.

### The container

- an argument names a CID the container does not carry
- a block that does not decode as what its field claims (a delegation
  where an invocation belongs, and the reverse)
- a duplicate or ambiguous CID
- blocks carried but named by nothing — refuse rather than store, since
  today `delegation_tokens` sweeps every token indiscriminately

### The recovery invocation

- carries any proofs. It must be self-signed and proofless, which is
  what makes it portable — and what makes verifying it without a
  revocation lookup sound. A chain with proofs would otherwise be
  accepted without its revocations consulted, so this is enforced rather
  than assumed.
- signature does not verify
- subject is not the custody DID the enrollment names
- command is not `/use/put/memory/cell`
- arguments name a space or cell other than the custody pair
- `checksum` does not match the carried `sealed` block
- `when` present — this must be a first write, not an overwrite of a
  cell someone has since rotated
- already expired, or expires before the activation window closes: the
  one check that cannot be deferred, since a link outliving its
  invocation strands the account exactly as before

### The consent

- signature does not verify
- issuer is not the custody DID
- audience is not the enrolling account — a consent given to one
  customer must not enroll another
- subject, when specific, is not the custody DID
- command does not cover `/consumer/provision`
- outside its validity window

### The link

- the finished URL exceeds the budget. Measured on the real URL, not the
  material, and before the row is written.

### The customer

- email is not a plausible address
- email already registered to a different account
- the custody DID is reserved by, or provided to, someone else
- the customer is already `Active`, or is `Suspended`

## Reservations

`consumer.reserved_until INTEGER` — one nullable column, total
interpretation:

- a timestamp: held until then, claimable after
- `NULL`: permanent — which is also what a claimed row looks like, so no
  separate status column and no "claimed" flag

Consistent with `suspend_until` in the same table, where null already
means indefinite.

`ADD_CONSUMER`'s guard widens: claim when the row is absent, when the
provider matches, or when `reserved_until` is non-null and past. A
`NULL` with a different provider is refused.

Custody reservations **must** expire. The custody DID is PRF-derived and
therefore stable, so a lapsed reservation is re-derived and re-reserved
by the same passkey on a new device — which is the recovery case this
whole design exists for. Holding it forever would strand it. There is no
squatting risk, since only the passkey holder can produce that DID.

## Steps

1. ~~Widen the `provision_or_defer` catch-all.~~ Done — `70681a7f`.
2. `reserved_until` migration; widen `ADD_CONSUMER`; reserve at
   enrollment.
3. Service derives the customer space DID and returns a `/use/get`
   delegation on it in the receipt. The client saves it the way joined
   spaces already save authority (`join.rs:986`,
   `profile.access().save(UcanDelegation(chain))`) and configures a
   remote subjected to it.
4. `Enroll` gains `recovery`, `consent`, `sealed`; verify without
   executing. Remove `deposit_scopes`, `verify_deposits`, and the
   `access` column.
5. `activation_link` carries the three named blocks, and the finished
   URL is measured before enrollment commits. Mint the link *before*
   writing the customer row (today the row lands at
   `registration.rs:245`, the link at `:455`) so an oversized one is a
   clean `Invalid` refusal with nothing committed — a row whose
   activation email can never work stranding the account is the failure
   this design exists to prevent. Measure the whole URL, not the
   material: origin, `/activate?ucan=`, and the activation invocation's
   own fields and signature all count against the budget.
6. **Enrollment writes everything; activation flips two fields.**
   Supersedes the "activation performs it in one transaction" shape
   above, which had to satisfy the presign gate mid-transaction. Instead
   enrollment writes, in one act:

   - the custody cell, through the service's own `BUCKET` binding
   - the customer row, `status = Registered`
   - the subscription rows, with `expires_at` set to the activation
     deadline

   Nothing is served, because `Registered` denies every read and write
   through the provider. So there is no window to fall out of and no
   gate to satisfy: the state is complete and simply inert.

   `/customer/activate` then carries the account and flips two fields —
   `customer.status = Active`, `subscription.expires_at = NULL`. A link
   that is never clicked leaves rows that expire on their own.

7. **Resend.** The did:web lookup already reports `PENDING`, so the
   login screen can offer a resend. A new self-issued command,
   `args: { account }`, guarded two ways: only while the customer is
   `Registered`, and no more often than a fixed interval — which needs
   a `sent_at` column to compare against. Neither guard is about
   authorization: the mail only ever goes to the address on the row, so
   the worst a caller achieves is mail to an inbox they do not control.

8. Remove client-side custody provisioning, the `PublishCustody` queue
   arm, and client-authored `AccountCustomer` writes.

9. Remove `deposit_scopes` and `verify_deposits`: the inert deposit
   check that compares a client's delegation against the function the
   client built it from.

## The container view (settled)

`InvocationChain::try_from` reads a `ctn-v1` container as an invocation
followed by delegations, and refuses any other token
(`dialog-ucan-core/src/container/invocation.rs`). Proven by execution: a
valid enrollment carrying the recovery invocation and the sealed
envelope was refused with `failed to decode delegation 2: Mismatch`
before any custody check ran.

Fixed in dialog rather than worked around here. `InvocationBundle`
(dialog `06be88d5`, branch `feat/invocation-bundle`) is a **second view
over the same bytes**: the invocation at the root, every other token
addressable by the CID it hashes to, resolved explicitly and typed at
the point of use — `resolve_invocation`, `resolve_delegation`, `block`.
`InvocationChain` is unchanged and still strict, so carrying blocks is a
different reading rather than a weaker one.

Note: a resolved invocation arrives **without proofs**, since a carried
block is a bare token. Fine here — the recovery invocation is proofless
by construction, and enrollment enforces that.

> **PIN**: tonk's 16 dialog deps currently point at
> `branch = "feat/invocation-bundle"`. This MUST return to a tag before
> the tonk PR lands. Drift from `tonk-2026-08-28` to main is two commits,
> and main's head is already tagged `tonk-2026-08-28b`.

## Open

- ~~Link size~~ — settled, measured. recovery 370 + consent 337 +
  sealed 64 = 771 raw, 1028 base64url, against a conservative ~2000
  character floor. The material rides in the link; no HTML-email POST
  and no server-side persistence needed. Pinned by
  `it_keeps_the_activation_link_inside_a_url_budget` in `tonk-identity`,
  so a later field addition fails the budget instead of quietly
  producing links that break in mail clients. Assumes container framing
  adds little beyond the blocks; worth re-measuring on a real assembled
  container.
- ~~Account reads a space it does not own~~ — settled, it already
  works. `RemoteConfiguration.subject` is explicit and overridable
  (`repository.rs:62`); a foreign space's local handle is built from a
  verifier-only credential parsed from the subject DID, no private key
  (`join.rs:1552`). Four live paths do this already — invite joins in
  worker and CLI, directory adoption, and profile main itself, which
  points at a remote subjected to the ACCOUNT DID (`account_state.rs:286`).
  No ownership check on the pull path; authority is a delegation chain
  walked to the subject and signed by the operator. Needs only the
  delegation retained and a remote configured.
- **Cell write from the handler** — possible, shape undecided. The
  presign path only signs, but the crate holds a `worker::Bucket`
  binding (`BUCKET` -> `tonk-spaces`, `wrangler.toml:25`) and already
  writes through it: `handlers/shortcut.rs:76` validates then `.put()`s
  with no permit. Same bucket the presign path signs against. Two
  caveats:
  - The service confines its own writes to a `tonk/link/` prefix on
    purpose (`shortcut.rs:30`). A cell write leaves that prefix for the
    keyspace permits are issued over — a real widening, to be chosen
    deliberately.
  - `Bucket` gives bytes at a key. The cell format (DAG-CBOR, block
    layout, head/revision) lives in dialog crates that are
    dev-dependencies only here. Transport is solved; encoding is not.

  So either pull `dialog-repository`/`dialog-artifacts` into the
  deployed worker, or have the service issue itself a permit and execute
  it — reusing the authorize path instead of duplicating the storage
  format. Undecided.
- Installs already broken need a fresh passkey ceremony. Not covered.

## Known limits of step 1

It stops consents being dropped, but recovers nothing already lost. And
a `Provision` entry that fails terminally *at drain time* still blocks
any publish behind it, since the queue cannot drop dead entries — steps
2-7 delete the queue, so no reaper was built.
