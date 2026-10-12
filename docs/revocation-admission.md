# Revocation admission barrier (BUG-138)

## Guarantee and boundary

After all removal receipts have succeeded against the updated service, subsequent
UCAN authorization and object-permit redemption refuse the removed grants and
all descendant authority. Each dependency retains the chain verifier's per-link
revoker scope. Unrelated grants and unrevoked leaves remain valid.

This is an **admission barrier**, not an in-flight-operation drain. A request
which passed its last authority check before removal may still complete afterward.
Downloaded bytes and local edits cannot be recalled. If the contract requires
that no already-admitted write commit after acknowledgement, a coordinator that
serializes revocation and mutation commits is still required; this patch does not
claim that stronger contract.

## Storage and compatibility

The existing `BUCKET` R2 binding stores monotonic, empty revocation facts under
`__tonk/revocations/{delegation CID}/{revoker DID}`. Ordinary storage capabilities
are scoped under subject DIDs and cannot address this reserved prefix. Never
expire, delete, or apply bucket lifecycle cleanup to these security facts.

R2 binding reads bypass the HTTP cache. Cloudflare documents strongly consistent
object reads and listing, globally following an acknowledged write:
https://developers.cloudflare.com/r2/reference/consistency/

`REVOCATIONS_KV` remains a legacy mirror. New facts are written to KV first, then
R2; acknowledgement requires both. If KV fails, the call fails. If R2 fails after
KV succeeds, the call fails and retry can complete the barrier. Existing R2 facts
are idempotent without another write. Replaying a valid legacy revocation also
records it in R2 before returning success. Legacy-only facts remain checked in
KV, preserving earlier removals; this does not retroactively eliminate the old
KV propagation window for old receipts.

All data-plane entry points and registration authority checks use the same
index. Missing bindings or failed lookups fail closed. No negative result is
cached in the service. A storage failure is not evidence of revocation.

New signed object permits include the exact revocation selectors observed while
verifying the invocation. Redemption checks them again before any object I/O,
including GET, PUT without a checksum, and DELETE. Legacy signed permits without
selectors return 401 `ReauthorizationRequired`; clients must reauthorize using
their original invocation. The pinned client automatically drops and retries a
cached GET permit once on 401/403. A noncached permit, including PUT/DELETE,
returns the rejection and needs an operation retry. This distinction is inspected
in dialog-remote-ucan-s3/provider.rs; automatic retry with a legacy deployed
client has not been run end to end. The server's legacy 401 behavior for all
three methods is covered by executed HTTP tests.
A direct subject-signed operation may legitimately have an empty selector list;
absence of the field is distinct from an explicit empty list.

## Rollout and rollback

No new binding, table, credential, or manifest migration is introduced. Deployment
is still an operational change and has not been performed by this implementation.
Complete the access-service rollout across all serving instances before exposing
the updated confirmation copy or promising the admission guarantee. Mixed old
instances can still authorize through stale KV and redeem legacy permits.

The KV-first mirror preserves new facts if code is rolled back, but rollback to
old code loses strong consistency and permit-redemption enforcement. Do not claim
the admission guarantee after rollback. Keep the new R2 facts permanently.

## Cost, latency, and required release validation

Each selector with at most four principals performs concurrent R2 HEAD reads,
then the legacy KV check on a miss. Broader selectors use paginated R2 and KV
listing instead of unbounded HEAD fan-out. Object redemption now performs these
checks on every network request. This adds storage requests, latency, and a
fail-closed dependency on R2/KV availability. No production latency or cost
benchmark has been performed. Measure representative chain depths, high fan-out,
large object streams, and cold/hot edges before rollout.

Native tests exercise fresh/direct requests, old read/write permits, ancestor
revocation, copied bearers, independent grants, partial leaf removal, repeated
revocation, missing legacy selectors, unchecksummed claims and index failures.
They do not establish live Cloudflare behavior. Before release, run disposable
Workers integration tests across locations with warmed negative KV results;
verify KV-first/R2-second failure and retry, paginated listing, R2 object lifecycle
rules, legacy-client retry, and no access through a remaining old deployment.
Use a slow in-flight PUT to confirm and document the admission boundary rather
than claiming it is drained by removal.

## Proof shape and URL bounds

The pinned verifier requires one ordered root-to-leaf proof sequence. It does not
search alternative branches. Only the invocation's referenced `prf` entries are
queried; unrelated proof material in the container is excluded. The regression
includes an unused, later-revoked proof path and verifies both its old permit and
fresh authorization still work.

Selectors grow with proof depth (candidate prefixes can make this quadratic).
The service refuses to issue a URL exceeding 16 KiB rather than truncating a
security dependency or returning an unusable URL. Such callers must use direct
UCAN access or a shorter chain. This is a representation limit for deep chains,
not a new permission. Cloudflare's documented request-URL bound:
https://developers.cloudflare.com/workers/platform/limits/

Measured URL sizes in native fixtures: an actual two-link grant produced 1,061
bytes; synthetic 2-, 3-, and 8-link chains produced 957, 1,370, and 4,617 bytes.
The 32-link synthetic case is explicitly refused at issuance. These fixtures
use compact did:key principals, not an upper bound on arbitrary DID lengths.
