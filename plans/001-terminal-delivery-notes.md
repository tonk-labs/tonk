# Terminal connection delivery

Implemented locally for plan 001 milestones 4–6. The contract below describes
the current implementation; dated checkpoints retain the sequence of evidence.
Hosted migration and release remain separate operator gates.

## Existing substrate

`tonk-cli/src/callback.rs` provides an ephemeral loopback HTTP one-shot callback.
`account.rs` and the worker ceremony use it for local account authorization. It
does not provide authenticated polling, remote/headless delivery, or additions
while a CLI is offline. `space_link.rs` is local ownership adoption. The access
service vault stores sealed custody recovery data and is not a connection inbox.

Delivery uses the existing access-service control D1 and its native `SqliteStore`
twin, additive migrations 0007–0009 and a narrow delivery trait. The disposable
production Worker harness verifies real D1 restart behavior. No connection row
participates in data authorization.

## Signed wire objects

- `LinkRequest`: version, random 256-bit nonce, exact recipient DID,
  creation time, short approval deadline, and display label. The CLI signs the
  canonical request with its retained recipient key; its hash is the request ID.
  It sends no private key.
- `Approval`: complete signed request, browser issuer, account proof, issue time,
  explicit decline flag and complete public grant bundles. The browser signature
  commits the entire selection. Subjects, grant CIDs and group IDs are verified
  from those bundles; the approving account is derived from its proof chain.
- `Addition`: initial request identity, exact recipient, unique delivery ID, new
  selected bundles, and issue time. It carries the same authenticated approving
  account and a currently valid browser device proof.

Use bounded canonical encodings, explicit versions, bounded bundle counts and
byte lengths, and reject unknown or conflicting fields. The implementations are
`tonk-invite/src/terminal.rs` and its `additions` module. Requests are bounded at
4 KiB, complete approvals/additions at 4 MiB, and each selection at 1024 spaces.
No selection is truncated to meet a limit.

## Authentication and publication

The initial approval is bound to the exact request, recipient key, and nonce.
The CLI independently validates service-trusted remotes, complete signatures,
subjects, scopes, expiry, recipient audiences, and the approving account proof.
It derives the approving account from verified authority, never an unsigned body
field, and pins that account for later additions. A delivery signature alone
does not establish account membership. Later additions require that same account
and current device authority at first publication, plus all ordinary bundle
checks. The CLI verifies the signed management envelope at its issue time and
every selected space grant at consumption time. Expired management proof alone
cannot shorten an independently valid space grant.

The mailbox stores immutable public signed deliveries addressed to the exact
CLI DID. Recipient-signed reads include the request/mailbox ID, short expiry,
and nonce. There is no account catalogue or space-list endpoint. Initial
publication is create-only for the request hash: identical retry is idempotent;
a different complete approval conflicts. The CLI stages and verifies the whole
initial selection before atomically publishing local registry entries.

Persist the addressed mailbox identity beyond the short initial approval
deadline so a browser can deliver additions while the CLI is offline. Delivery
IDs and read cursors are transport bookkeeping, not capability activation.
Cancellation/timeout prevents local acceptance without mutating configuration;
it never silently revokes already issued grants. Grant lifetime
remains separate from the approval deadline. Standard revocations continue to
govern access independently of delivery records.

## First executable slice

Prove one signed request, one complete browser approval, recipient-authenticated
polling, and atomic CLI import using the real SQLite adapter. Reject wrong-key
reads, request substitution, altered selection, replay with different bytes,
expired pending requests, and another account's additions. Then restart the
file-backed SQLite service and the production D1 Worker to prove offline delivery
survives restart. Add multi-space approval only after this slice passes.

## Refinement: no durable pending-request write

Prefer creating and signing the bounded request entirely on the CLI. The trusted
approval URL carries the public signed request (prefer a fragment to avoid
ordinary request/referrer logging). Authenticated polling of an unknown request
hash returns no delivery and creates no row. The browser's first publication
carries both the complete signed request and the signed complete approval. The
service verifies the request signature, exact recipient, bounded deadline using
service time, authenticated current account/device authority, and grant bundles
before its first create-only write. Account context is therefore available for
write quotas; a CLI key alone cannot create persistent pending rows. Polling
still needs bounded reads and abuse limits.

This fits the required cancellation/configuration guarantee with an explicit
limit: cancellation is local. Persist distinct pending/interrupted, cancelled,
and completed request outcomes. A cancelled request must never import an approval
or silently resume, even if a browser published concurrently; retry starts a new
nonce. Without a server cancellation tombstone, the service cannot observe local
cancellation and may accept publication before the signed deadline. That can
leave addressed public grants in the mailbox, but changes no CLI aliases or data
and grants no other reader access. Do not claim remote withdrawal or revocation
from cancelling the local process.

Reject first publication after the signed approval deadline. An identical retry
of a complete approval already durably published is a read/idempotent response,
not a late new grant issuance. Define interrupted-request recovery separately
from cancellation: if resuming an earlier valid approval is supported, the CLI
must still verify all grant expirations and stage the full bundle atomically.
An expired pending attempt must not automatically accept a later approval.

The first durable approval also fixes the mailbox recipient and authenticated
approving account for offline additions; later delivery does not need the initial
approval window to remain open. Account pinning is derived from verified proofs,
not request/body metadata. First-writer conflict remains a potential availability
failure if a public request is copied into a different authenticated account's
approval flow; it must not bypass CLI approval-principal validation or overwrite
an existing complete selection. The initial account-selection trust boundary
must be exercised by a wrong-account browser test.

Add tests for unknown-hash polling with no rows, cancelled-versus-published
races, deadline boundary publication, identical versus conflicting replay, and
offline additions after the original request deadline. A server-side cancellation
promise would require a separately authenticated cancellation record and is not
part of this minimal design.

## First storage slice evidence (2026-09-16)

Implemented migration `0007_connection_delivery.sql`, a separate `DeliveryStore`
trait, and native SQLite/production D1 adapters. One SQL write publishes the full
approval and recipient/account address with create-only conflict handling; no
pending-request write exists. Account limits are 1,024 initial approvals and
64 MiB total decoded payload, checked inside the same insertion statement.

`cargo test --offline -p tonk-access-service --features helpers --lib
connection_delivery_storage -- --test-threads=1` passed the real file-backed
SQLite test: complete immutable publication, exact-byte retry, conflicting bytes
and recipient refused, unknown polls create no records, exact-recipient read,
and close/reopen returns unchanged complete bytes. The storage test uses opaque
fixture bytes and does not establish signature authentication or HTTP behavior.
`cargo check --offline -p tonk-access-service --target wasm32-unknown-unknown`
passed the D1 adapter compile. The subsequent authenticated HTTP and D1 checks
are recorded below.

The next slice passed with the shared signed codec and ordinary UCAN publication
invocation. `connection_delivery_http_authentication_replay_deadline_and_revoked_authority`
passed against the actual native HTTP service: complete approval, exact-recipient
read, wrong-key isolation, immutable retry/conflict, expired first publication,
tampered read, and rejection of a new approval after standard device revocation.
The exact already-stored approval remains an idempotent receipt retry.

`sh scripts/test-terminal-delivery-worker.sh` freshly built the production Wasm
handler and passed the disposable Miniflare D1/KV fixture. Unknown reads and an
unregistered publisher left no rows; approved bytes survived dispose/reopen;
wrong-key reads, conflicting decisions, expired requests and revoked publisher
authority were rejected. The store's three focused tests also passed, including
two independent SQLite connections racing complete conflicting publications and
quota enforcement preserving existing receipts and unrelated accounts.

The endpoints are `POST /connection/delivery` (signed CBOR approval, JSON
`requestId`/`recorded` receipt) and `POST /connection/read` (signed recipient read,
raw CBOR approval or empty 204). Both adapters bound streaming bodies and disable
response caching. These local service checks do not establish CLI/browser
selection, cancellation, multi-space import, hosted deployment or global
propagation behavior.

## Delivery capacity and lifecycle follow-up

The small native/Miniflare payloads above do not establish the complete 4 MiB
format limit. [Cloudflare's D1 limits](https://developers.cloudflare.com/d1/platform/limits/)
(checked 2026-09-16, page updated 2026-04-21) cap each string, BLOB, or table row at
2,000,000 bytes. A single hex-encoded row therefore cannot implement the format's
maximum. The required adapter follow-up is bounded payload chunks committed
atomically with their immutable parent, preserving exact bytes and the full
format limit. It needs an actual production-D1 fixture above 2 MB, not only an
SQLite or small local sample. No deployment capacity claim is made yet.

Account deletion must also remove the service's account-associated initial and
addition copies (including future chunk children), preserving every other
account, the ordinary revocation index, and client-retained grants/local data.
This belongs in the existing account-purge transaction and is an explicit
compatibility gate, not a blanket retention exception for public proofs.

### Chunked storage checkpoint (2026-09-16)

Migration `0009_connection_payload_chunks.sql` adds parent payload-size metadata
and bounded ordered child rows. Both SQLite transactions and D1 batches write
all children with the immutable digest/length parent atomically. Quotas count the
complete payload, and reads verify ordered chunks against the complete digest
and length before returning bytes. Legacy inline rows remain readable after the
migration. The signed wire format and 4 MiB complete-payload cap are unchanged.

The fresh focused native command `cargo test --offline -p tonk-access-service
--features helpers --lib connection_ -- --test-threads=1` passed six tests. This
includes 3 MiB payloads for initial approvals and additions, file restart and
exact replay, independent SQLite connections racing large conflicting complete
publications, forced child-write rollback with no visible parent or children,
corrupt partial-content rejection, and the version-8 inline migration. It also
proves completed account deletion removes only that account's mailbox parents
and chunk children while preserving another account's data; an active customer's
failed deletion leaves its mailbox intact. Production D1 large-payload and full
authenticated account-purge evidence remains pending at this checkpoint.

The production follow-up now passed: `sh scripts/test-terminal-delivery-worker.sh`
freshly compiled the Worker and exercised actual disposable D1/KV/R2 with 334
selected spaces. The complete signed approval was 2,111,908 bytes and the addition
2,111,941 bytes. Concurrent identical publication returned one create and one
existing receipt; a concurrent read was complete or absent. Conflicting approval
was rejected; exact initial and addition bytes survived dispose/reopen. An
injected second-chunk D1 trigger failure rolled back both parent and children.
The standard authenticated customer-purge HTTP command removed its initial and
addition copies and chunk children, preserved another account's exact deliveries
and the existing standard revocation KV fact, and succeeded on repeat. The
migration/schema documentation checks passed three tests. These are local
production-handler results, not hosted rollout or global propagation evidence.

Final lifecycle review exposed and reproduced a late-write race: the publisher
could pass the pre-insert customer check, account purge could finish, and the
storage adapter could then create a new initial mailbox. The focused SQLite
regression failed with `Created` after purge. The insertion statement now also
requires the same customer to remain `Active` inside the atomic write. This uses
existing customer lifecycle state and adds no tombstone or data authorization
record. A lost lifecycle race reports account unavailable rather than recreating
metadata; additions already require the surviving addressed initial parent.

Rollout compatibility: the new adapter reads existing inline rows, but an older
service binary cannot decode newly written chunk references. Migration 0009 must
be applied before enabling the new writer; rollback must retain a chunk-aware
reader. This is mailbox availability compatibility, not a change to grant
validity or the shared CLI wire format. No hosted migration has been applied.

Final checks after the atomic lifecycle guard passed: the required
`cargo test --offline -p tonk-access-service --features helpers connection --
--test-threads=1` ran six storage, four ordinary grant/revocation HTTP, and one
signed-delivery HTTP test successfully. `--lib deletion` passed seven existing
lifecycle tests. The final `sh scripts/test-terminal-delivery-worker.sh` rebuilt
the guarded production Worker and passed the 334-space large-payload D1 test,
including refusal to republish after account purge. Schema checks passed three
tests and `git diff --check` was clean. No hosted migration, deployment, or global
propagation verification was performed.
