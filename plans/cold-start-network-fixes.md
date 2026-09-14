# Cold-start network fixes

Implemented 2026-09-10 from `cold-start-network-diagnosis.md`.

The worker memoizes successful manifest verification and its path/hash map for
its lifetime; concurrent failures share a rejection and allow later retry.
Foreground asset loading and generation installation share in-flight verified
responses. Each consumer receives a clone. A build-and-manifest-specific
`TONK_DOWNLOAD_*` cache retains successful bytes independently of publication.
Persisted bytes are rehashed before reuse, including after worker restart.
Storage failure does not prevent online consumption of verified bytes, but
atomic offline publication still requires successful staging and final writes.

The existing building/publishing/adopted transaction remains the authority for
complete offline availability. Failed attempts retain download inputs; adoption
removes that cache. Obsolete download caches join the existing generation-aware
cleanup after adoption. No retained adopted generation is repaired in place.

First-use background preparation waits for the bootstrap's existing mounted-page
check to send `content-ready`, then for five seconds without active foreground
asset requests. A 60-second deadline provides a fallback if the mounted signal is
lost or activity continues. The mounted signal is a scheduling hint, not proof
that every nested view rendered. Bulk downloads use two lanes. Successor install
still completes its generation before replacing the incumbent. Source maps are
excluded from the stamped automatic offline graph, while remaining on disk for
explicit developer requests.

## Validation

New shipped-source tests cover concurrent guests and fill sharing one acquisition,
manifest failure/retry, independently consumable responses, interrupted fill
reuse across worker restart, corrupt partial bytes, and optional retention quota
failure. Existing tests retain publication-failure, interrupted staging,
deployment/hash mismatch, and incumbent cache protections. The publisher fixture
now includes a source map and verifies its exclusion from the exact graph.

An isolated headless Chrome used a temporary copy of the existing dev artifact,
with the changed worker and mounted notification restamped into it. This was not
a fresh Rust build. Nested Welcome content rendered, and the cache marker reached
`adopted`. Server logs recorded 899 requests and 32,357,496 response-body bytes
(using local file sizes for successful 200 responses): one manifest and one guest
Wasm acquisition for two guest loads plus offline preparation. UI Wasm still had
two requests because initial uncontrolled page loading precedes worker mediation.
The table engine's offline transfer began at approximately navigation +10.59s.
The generated manifest contained zero source maps; two explicit PostHog map
requests still appeared outside that graph. These are local uncompressed results,
not a controlled before/after timing comparison.

Chrome Slow 4G emulation was rejected because it conflicts with the skill-required
URL blocklist. Constrained-bandwidth scheduling, production/CDN behavior, Safari,
and device tests remain unverified. The five-second quiet period, 60-second
fallback, and two lanes are initial conservative policy values, not measured
optimal settings.
