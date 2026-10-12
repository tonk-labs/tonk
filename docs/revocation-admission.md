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

## Supported protocol and CLI upgrade gate

Data operations require the original UCAN in Authorization (`ucan-direct-v1`).
Legacy CBOR data redemption returns HTTP426 with `Rejection::Unclassified` and
an actionable `detail`; `code: UpgradeRequired` provides an additional machine
label. This existing rejection shape is intentional: released old clients cannot
parse a newly invented rejection variant. Registration, deletion, purge and
revocation CBOR dispatch before this gate and remain supported.

Missing version metadata is normal and accepted for direct requests. Legacy
requests are rejected even if they claim a new version. No version header grants
authority: every direct request still verifies its original chain and signed
arguments. Signed range selects returned bytes; unsigned HTTP Range cannot change
it. There are no issued descriptors, cached-permit range retries, invocation-ID
query adapters, or independent bearer authority.

Authentic npm v0.6.15 `tonk pull` and `tonk push` print the 426 detail without
`--verbose` and exit4. The probe uses a disposable container, not a user's profile.
The unchanged Linux binary needs the container's explicit loader because its
published Nix interpreter is unavailable; that workaround tests error rendering,
not installation portability. An unconfigured `account sync` is not evidence of
linked-account error rendering. The production Worker fixture can invoke the real
CLI using `TONK_RELEASED_CLI_FIXTURES`; see `scripts/cli-upgrade-probe.cjs`. The test wrapper never modifies the executable.

Supported admission is defined by the direct protocol, not semver ordering.
The earliest stable CLI verified to send this protocol is0.6.16;0.6.15 sends
obsolete CBOR redemption. Both0.6.16 and0.6.17 were exercised as real binaries.
The recommended **macOS arm64** CLI is the verified npm `@tonk/cli@0.6.17` artifact;
run `tonk --version` and check PATH afterwards. npm `stable` remains intentionally
frozen at0.6.11 and must not be recommended. `tonk update` is not a universal route:
npm/Nix-managed installations use their own package manager.

**Deployment prerequisite: publish a portable Linux CLI release first.** npm
0.6.17 uses direct UCAN but still embeds the unavailable Nix loader. The later
immutable `tonk-kdp1zi8hiz1j` release predates the musl fix too. The current tested
candidate is #1085 commit `06c18e3c3ae87db19bfc8eab27bceb0acebcf103`, including
musl commit `c68f2886d0` and Debian/Alpine artifact gates. A release owner must choose
a new version newer than0.6.17 from an approved commit containing these changes,
run both published tarball and ordinary npm-install smoke gates, publish through
the normal release process, then replace the Linux-pending guidance with that
verified release/install target before deploying this Worker. No release or
deployment is part of this PR task. Until then the error explicitly says Linux
has no verified ordinary npm upgrade; it never recommends the broken017 binary.

Old `/object/*` remains typed401 rather than404. Cached old GETs can invalidate
and reach the readable426 gate. Already-held old PUT/DELETE URLs may expose only
an HTTP failure because those clients discard response bodies; retrying a normal
CLI push/pull reaches the upgrade gate. This limitation cannot be repaired inside
an already released binary. Removed grants remain denied; no compatibility
fallback restores bearer execution.

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
length only on successful storage. Rejected obsolete requests transfer no objects.

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
only after storage; rejected payloads and infrastructure failures are not
successful uploads.

## Validation and rollout limits

`tests/connections.rs` covers direct execution and obsolete-protocol rejection,
partial removal, descendant/copy authority, independent chains, unsupported
escalation, expiry, payload validation, ranges, empty uploads and CAS.
`upload.rs` tests truncation, extra data, read errors, both digests, zero bodies
and final-byte retention. `scripts/test-connection-worker.sh` exercises the actual
production Wasm against disposable Miniflare R2/KV/D1, including absent KV mirrors,
restart, storage failure, metering and a 33MiB upload. The optional CLI probe runs the authentic published v0.6.15 executable.

These tests do not establish live multi-location Cloudflare behavior. Release
validation still needs warmed legacy KV misses across locations, storage-write
failure/retry ordering, pagination, lifecycle rules and representative latency
and cost. Full rollout is necessary before promising the admission barrier;
mixed old instances and rollback to bearer-serving code lose the guarantee.
No merge, production deployment, credential rotation or binding change is part
of this PR.
