# Hosted agent peer: Workers spike (stage 0)

Status: spike done 2026-10-29. Answers open question 3 ("Workers fit") of
`plans/2026-10-28-hosted-agent-peer.md` (PR #1200), as far as it can be
answered on localhost.

## Verdict

**Conditional go, on the paid plan.** The ordinary tonk worker runs in a
Workers isolate, rebuilt from nothing on every request, and every operation
stage 0 lists works through the access service. The bundle fits the paid
limit at half of it. A targeted request costs about 60 to 80 ms of fixed
overhead and 100 to 180 ms of CPU, and stays under 32 MiB of wasm memory.

Three conditions come with the go. Stage 2 has to meet them:

1. **Bound what one request reads.** A query that returns a whole concept
   from a 20k-record space peaks at 231 MiB of wasm memory, well over the
   128 MB isolate limit. Answers have to be capped or paged, `transact=false`
   should not compute the matches twice, and nothing may materialize a whole
   space.
2. **Fix concurrent writes from one session.** Five concurrent writes
   through one token all returned 200, and only one of them landed. The
   other four were lost without any error. The design assumes that
   "concurrent writes resolve at the revision cell", and that does not hold
   when the writers share a session key.
3. **Feed subscriptions from `live.rs`.** An SSE subscription delivers its
   first snapshot in 63 ms. It never sees a write made by another request,
   because the stateless peer has nothing that tells it about remote
   commits. This was already planned for stage 2, and the spike confirms it
   is required.

**No-go on the free plan.** The bundle is 4.96 MiB gzip against a 3 MB
limit, and a point query takes about 130 ms of CPU against a 10 ms limit.

If condition 1 or 2 turns out to be unfixable inside an isolate, the plan's
fallback still applies (a container, stateless per call). Nothing measured
here argues for that yet.

## What was built

- `rust/tonk-worker`, feature `volatile-storage` (it does nothing on native):
  - on wasm, `DefaultSpace` becomes dialog's in-memory `VolatileSpace`
    instead of IndexedDB/OPFS;
  - `device::open_system` keeps the system credential store per
    directory. In-memory storage forgets everything between opens, and the
    registry opens the system key three times while booting;
  - `router::hosted::HostedPeer` opens the ordinary worker acting with a seed
    and holding given proofs, mounts a space as a replica of a remote, serves
    requests with the unchanged worker router, and closes.
- `update_pending()` no longer assumes a service worker registration. A
  Workers global passes `instanceof ServiceWorkerGlobalScope` but has no
  `registration`, so every SSE query trapped the isolate. This is the only
  change to existing behaviour, and the browser keeps the same result.
- `prototypes/agent-peer-worker`: a workers-rs Worker (worker 0.8.5, `http`
  feature) built with `worker-build --release` (the browser worker's
  profile: opt-level s, fat LTO). It is not a workspace member. For every
  request it decodes the token, opens a `HostedPeer`, mounts the agent space
  with the access service as its remote, pulls, calls the worker route,
  pushes if the request wrote, and drops everything. Each response carries
  `x-spike-timing` (milliseconds per phase, wasm memory before and after).
  `drive.mjs`, `fresh.mjs` and `concurrent.mjs` produced the numbers below.

Shortcuts:

- **Token.** The token is `base64url(seed).base64url(powerline container)`,
  not the stage-1 UCAN container. The powerline is
  `mint_addressed_device_delegation(agent space -> session)`, the shape the
  design names. `/_spike/setup` mints it from hardcoded seeds (blake3 of a
  fixed string) and provisions the space on the access service through
  `/_test/provision`. The request path checks the seed, the audience and the
  powerline shape, and never sees the agent space's key. Built on `main`, not
  on #1202.
- **List spaces** answers from the token alone (the powerline's issuer).
  Invited spaces and contacts are not covered.
- **Access service.** This is `tonk-access-local`, the native emulation the
  `run-app` skill uses, not the access service under `wrangler dev`. It has
  the `/_test/provision` shortcut, and running the real one would have needed
  D1, KV, R2 and Durable Objects bootstrapped in miniflare. The hosted
  Worker itself runs in workerd under `wrangler dev` (wrangler 4.140.0,
  workerd 2026-09-23).

## Operations

| Operation | Result |
| --- | --- |
| List spaces | Works, from the token only |
| Query (notation, `evaluate?transact=false`) | Works |
| Evaluate (`transact=true`, then push) | Works when sequential; see concurrency |
| Subscribe (`query` with `Accept: text/event-stream`) | First snapshot works (63 ms); no updates from other writers |
| Store asset (`blob` POST, then push) | Works: 256 KiB, 1 MiB and 4 MiB |
| Read asset (`blob/{entity}`) | Works, bytes identical |
| Bad, mismatched or missing token | 401; a space with no invite gives 403 |
| Not tried | `Range` reads, revocation, lapsed invites, other spaces, contacts |

Noise in the logs that did no harm: `BroadcastChannel is not defined` (the
worker logs it and goes on), and each boot tries to fetch
`/library/profile.yaml` relative to a global with no base URL and skips it.
Each request also commits a few profile meta facts while it boots. That is
wasted work for a peer that is dropped at the end of the request.

## Method

- **Synthetic space.** Written through the Worker itself: a 4-attribute
  concept (title of about 40 B, body of 60 words or about 400 B, natural
  rank, one of 50 tags) and 20,032 records, about 80k facts and about
  9.6 MB of attribute values. They went in as 1,000-record batches plus
  about 100 single-record writes. Afterwards the remote held 3,419 objects
  and 190 MB, history included. This is the largest space that could be
  built here in reasonable time. How it compares with the largest real
  space is unknown, because there was no access to production data.
- **Memory** is the wasm linear memory (`memory.size`). It never shrinks,
  so it is the isolate's high-water mark. `fresh.mjs` restarts
  `wrangler dev` for each operation and reads the memory after a cold and a
  warm run. The JS heap is not included: workerd does not report it
  locally, and the 128 MB limit covers both.
- **CPU** is utime plus stime of the workerd process serving the Worker,
  from `/proc` (10 ms ticks), averaged over 20 warm runs. It includes
  workerd's own HTTP handling, the JS glue and the outbound fetches, so it
  overstates the CPU Cloudflare would bill.
- **Wall times** are on a shared 4-core box with other builds running.
  Phases are measured inside the Worker with `Date.now()`, which only
  advances on I/O in production but runs freely in local workerd.

## Numbers

### Bundle

| | Raw | Gzip |
| --- | --- | --- |
| `index_bg.wasm` | 17.0 MiB | 4.90 MiB |
| `index.js` (glue) | 46 KiB | 12 KiB |
| `wrangler deploy --dry-run` total | 17,489 KiB | **5,078 KiB** |

That is 50% of the paid limit (10 MB) and 165% of the free limit (3 MB).
No size work was done (no `opt-level = "z"`, no route pruning), so there is
headroom. The module uses the wasm exception-handling `exnref` encoding:
workerd compiles it, and Node 22's V8 refuses it. Deploying therefore needs
a current workerd/V8 on the edge, which a real deploy would confirm.

### Cold start (local)

- `wrangler dev` reaches ready in about 3.3 s. That is tooling time, not
  isolate time.
- `GET /` (no peer) takes 55 ms on the first request and 8 ms afterwards.
- The first real request in a fresh isolate takes 500 to 700 ms wall. Of
  that, `open_peer` takes 150 to 220 ms against 12 to 25 ms warm, which is
  V8 tiering up through Liftoff and TurboFan. The second request takes
  about 200 ms, and later ones take 110 to 150 ms.

### Per-request phases (warm, point query, 20k-record space)

| Phase | ms |
| --- | --- |
| decode token (WebCrypto import) | 1 |
| open peer (keys, profile, session, boot) | 12 to 25 |
| mount replica | 13 to 25 |
| pull (lazy: fetches the head, not the space) | 22 to 40 |
| query | 45 to 65 |
| close | 0 to 1 |

The pull costs the same at 10 records and at 20k: the replica reads tree
nodes from the remote as a query touches them. Statelessness costs about 60
to 80 ms of fixed overhead per request, and most of that is round trips to
the access service.

### Memory and CPU per operation (20k-record space)

| Operation | Peak wasm memory (fresh isolate) | CPU per op (warm) | Wall p50 (warm) |
| --- | --- | --- | --- |
| Idle isolate | 1.8 MiB | | |
| Point query (one record) | 11.7 MiB | 134 ms | 137 ms |
| Evaluate one record + push | 17.1 MiB | 174 ms | 186 ms |
| Store 1 MiB asset + push | | 150 ms | 193 ms |
| Store 4 MiB asset + push | 31.9 MiB | | 357 ms |
| Read 1 MiB asset | | 108 ms | 114 ms |
| Read 4 MiB asset | 23.9 MiB | | 201 ms |
| Filter by tag (400 of 20k match) | 57.8 MiB | 1,939 ms | 3,291 ms |
| Scan one field of all 20k (2.2 MB answer) | 90.9 MiB | | 1,030 ms |
| Whole concept, all 20k (16.8 MB answer) | **231.2 MiB** | 3,402 ms | 3,734 ms |
| Write a 1,000-record batch | grew to about 100 MiB by 18k records | | 3.6 to 6.1 s |

- The whole-concept scan returns every field twice (`matches_before` and
  `matches_after`), which roughly doubles its cost. Even halved it would not
  fit in 128 MB.
- The tag filter is slow because nothing indexes the value, so it walks
  much of the tree.
- Batch writes get slower as the space grows (3.1 s at 1k records, 5.1 s at
  18k). An agent writing a few records at a time pays the evaluate-one row.

### Concurrency

- **One session, 5 concurrent writes:** all five returned 200, and every
  `revision_after` had the same issuer and edition (98). One record
  survived. The other four were lost without any error. Sequential writes
  through the same token all survive: all 21 single-record writes of the
  CPU run were read back.
- **Three sessions of one agent, 6 concurrent writes:** 3 returned 200 and
  3 returned 502 with `SYNC_CONFLICT` once the push retries ran out. Two
  records survived: the two sessions-0 writes again collapsed into one, and
  the conflicts were reported as errors.

The second case is the honest behaviour: a write that did not land says so.
The first case is the problem. Every peer built from one session key signs
revisions as the same issuer, and concurrent revisions with the same issuer
and edition are treated as one. Possible fixes are a fresh issuer per
request, serializing writes per session (a Durable Object per agent), or a
merge in dialog that tells same-issuer forks apart. Stage 2 has to choose
one.

## What cannot be determined locally

- **Production CPU accounting.** Billing counts isolate CPU only. The
  numbers above include workerd's HTTP stack and are an upper bound. The
  paid default limit (30 s, configurable) is far above any targeted
  request. Whether scans would hit a configured lower limit depends on the
  configuration.
- **Production cold start.** That means fetching the script, compiling 17
  MiB of wasm, and the startup limit on global scope. The global scope here
  only instantiates the module, but a deploy would show whether Cloudflare's
  startup check accepts it.
- **The JS heap's share of the 128 MB limit**, and whether Cloudflare kills
  the isolate at the first request over the limit or only later. Locally,
  231 MiB ran without complaint.
- **Round-trip cost to the access service in production.** Mount and pull
  each cost one or two round trips. With smart placement the Host Worker
  and the access service should share a colo. A request that misses would
  pay that per request.
- **Real space sizes** and the read patterns real agents use.
- **How often isolates are reused.** Every warm number here comes from a
  reused isolate. Cache whatever is safe to cache: a verified powerline,
  per the design, and possibly the compiled module and opened keys.

## Next, for stage 2

- Cap answers (rows and bytes) and page large ones. Stop `transact=false`
  from computing matches twice for plain reads.
- Pick and test a fix for same-session concurrent writes before building on
  "concurrent writes resolve at the revision cell".
- Skip the per-request profile bootstrap commits and the
  `/library/profile.yaml` fetch for hosted peers.
- Feed SSE from the space's `live.rs` socket.
- Deploy the spike to a preview account once, to replace the local cold
  start and CPU numbers with real ones.

## Reproduce

```bash
# access service (native, as in the run-app skill)
cargo build -j 2 -p tonk-access-service --bin tonk-access-local --features helpers
ACCESS_STATE_DIR=/tmp/access target/debug/tonk-access-local   # prints ACCESS_SERVICE_URL

# the Worker (needs worker-build 0.8.5, wasm-bindgen 0.2.126, wasm-opt, esbuild)
cd prototypes/agent-peer-worker
worker-build --release
wrangler dev --port 8788 --var ACCESS_SERVICE_URL:<url>

node drive.mjs setup            # token -> .token
node drive.mjs seed 20010 1000  # synthetic space
node drive.mjs ops              # every operation once
WORKERD_PID=<pid> node drive.mjs cpu 20
ACCESS_SERVICE_URL=<url> WRANGLER=$(which wrangler) node fresh.mjs
node drive.mjs setup 1; node drive.mjs setup 2
node concurrent.mjs 6 .token,.token1,.token2
```
