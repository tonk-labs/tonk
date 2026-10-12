# Direct UCAN admission (BUG-138)

Every data operation verifies the original signed UCAN chain at execution:
proof scope, ancestor expiry, signatures, entitled revokers and provisioning.
Request-local operation metadata is not a credential and has no independent
expiry. The service no longer issues or accepts HMAC object authority.

After all removal receipts succeed against updated instances, subsequent
admissions reject that grant and descendants. Independent grants remain valid.
Already-admitted operations can finish; this is not a drain or a guarantee that
no write commits after the removal receipt. Downloaded data cannot be recalled.

## Revocation storage

The existing R2 `BUCKET` stores monotonic empty facts under
`__tonk/revocations/{delegation CID}/{revoker DID}`. Subject-scoped capabilities
cannot address this service prefix. These facts must never expire or be covered
by object lifecycle deletion. They bypass the object cache.

New facts are mirrored to `REVOCATIONS_KV` first, then R2; acknowledgment requires
both. A failed write returns failure and can be retried. Existing R2 facts are
idempotent. Legacy KV facts remain consulted so earlier removals are preserved;
this cannot retroactively remove the old propagation window for old receipts.
Lookups fail closed on infrastructure errors, without calling them revocations.
The pinned verifier's per-link candidate scope is retained; unrelated proof
material does not become an authority dependency.

## Transport compatibility

Modern Authorization-header requests execute directly. Legacy CBOR redeems
return the existing Permit-shaped transport description: POST `/ucan/`, the
**original** signed invocation in Authorization, and its full invocation CID in
the query. The query only separates old clients' URL-keyed read flights; it never
authorizes. Each execution verifies the chain again, so timely redemption cannot
extend ancestor expiry or evade subsequent removal. Unsupported or malformed
operations never fall back to bearer authority.

Old `/object/*` URLs return typed 401 `ReauthorizationRequired`, including GET,
PUT and DELETE. Cached legacy reads invalidate and redeem once. In-flight older
writes/deletes are not automatically retried and may surface one operation
failure across cutover; callers must retry according to the operation's CAS
semantics. This cutover was explicitly approved; no deployment is authorized.

Legacy read caches omit Range. A descriptor executes only its signed range.
A mismatched requested Range, including absent versus present, returns 401 so a
cached read redeems the current invocation. An old client's **concurrent cold
reads of different ranges of the same blob** can share one redeem while neither
is marked cached: the mismatching read is safely rejected and is not retried by
that old provider. The standalone v0.6.15 probe documents this case, alongside
sequential cached-range recovery and concurrent distinct-object reads. The failure
is `Authorization(UnavailableProof { link: "blob read failed: 401 Unauthorized" })`;
an explicit retry on the same client recovered the exact requested range in both
fixture variants. Earlier
or external clients are not covered by running that probe. v0.6.16/17 and current
clients default to Direct; explicit legacy transport remains subject to its
cache behavior. Do not promise transparent recovery for all old reads.

## Streaming, cache and response semantics

Known-length uploads have no new application upload cap. Signed blob size also
supplies the length for headerless uploads. The Worker forwards chunks with
backpressure, retaining one final byte until EOF, declared and signed sizes,
SHA-256 and BLAKE3 validate. R2 never receives a complete invalid body. Empty
uploads validate before storage. Incoming failures, truncation and extra data
abort the stream; validation is never implemented as overwrite-then-delete.
Headerless archive/cell uploads retain the historical direct transport's collect
fallback. The collected bytes must be nonempty (otherwise 411), and all signed
checksums/digests validate before storage; invalid bodies never replace an object.
This narrow fallback retains the preexisting **unbounded-buffer risk**. It needs
a separately designed limit or spooling solution; not all uploads use bounded
memory. The historical 32MiB check only inspected optional Content-Length and
never bounded this headerless path. Known-length and signed-size uploads use the
streaming validator without a new 32MiB cap. Metering records the actual collected
length only on successful storage, and descriptor issuance records no transfer.

Platform/R2 limits still apply. The native test helper shares the validator;
its local S3 reads and uploads remain buffered and do not prove Worker memory use.
The LocalS3 fixture accepts truncated request bodies; the helper validates uploads
fully before sending them there. Worker atomic-abort evidence comes from the
production Wasm/R2-binding fixture, not the native S3 server.

Archive/blob data caches are accessed only after authorization and provisioning.
Keys encode the entire raw object key in a versioned namespace. Fills use a
separate R2 response stream, without a Rust whole-object buffer or response tee.
As before, bypass skips lookup but still fills. Memory cells are never cached.
Ranges, ETags, missing404, CAS412 and POST/Authorization CORS remain supported.
Conditional DELETE retains the existing R2 HEAD-then-DELETE limitation: it is not
an atomic compare-and-delete. Upload bytes are recorded at successful execution,
not descriptor issuance; rejected payloads and infrastructure failures are not
successful uploads.

## Validation and rollout limits

`tests/connections.rs` covers fresh/direct and original-descriptor execution,
partial removal, descendant/copy authority, independent chains, unsupported
escalation, expiry, payload validation, ranges, empty uploads and CAS.
`upload.rs` tests truncation, extra data, read errors, both digests, zero bodies
and final-byte retention. `scripts/test-connection-worker.sh` exercises the actual
production Wasm against disposable Miniflare R2/KV/D1, including absent KV mirrors,
restart, storage failure, metering and a 33MiB upload. The optional standalone
legacy probe runs pinned v0.6.15 client code, not a rewritten client emulator.

These tests do not establish live multi-location Cloudflare behavior. Release
validation still needs warmed legacy KV misses across locations, storage-write
failure/retry ordering, pagination, lifecycle rules and representative latency
and cost. Full rollout is necessary before promising the admission barrier;
mixed old instances and rollback to bearer-serving code lose the guarantee.
No merge, production deployment, credential rotation or binding change is part
of this PR.
