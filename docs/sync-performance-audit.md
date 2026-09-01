# Sync performance audit: why joining a space takes a while

Status: audit record, 2026-09-01. Companion to dialog-db's
`notes/sync-performance.md` (measurements, soak harness, dialog-layer
findings); this note covers the tonk side of the join and sync paths.

## Summary

Joining is slow on real networks for reasons that are invisible locally:

1. **Every cold block read costs two sequential HTTP round trips plus a
   signature** — a per-object permit redeem at the access service, then
   the storage GET. The permit cache is keyed per object path, so a
   first replication *never* hits it. Measured in the dialog soak: on a
   4G-class link this alone accounts for ~half of join latency.
2. **The join fetches everything twice.** `stage_join` runs its pull,
   validation reads, and roster reads against a **throwaway volatile
   pool** (`router/join/staging.rs`); `install_claim_nodes` then copies
   only the claim's novel nodes into the durable replica. Every block
   the staging phase hydrated is dropped when `Staging` drops, and the
   first render re-fetches all of it through the durable replica's
   remote.
3. **The join's reads were serial.** Three roster queries
   (`claim_changes`) ran back to back, each a cold root→leaf descent;
   `validate_content` runs twice (before and after the claim commit —
   the second pass is warm, so it is cheap; the first is not).
4. The lazy-join architecture itself is sound: `pull` adopts the remote
   head by root hash with zero block reads, and only the nodes a page
   actually touches are fetched. The eager alternative measured at
   ~8.5 s for a modest 18 MiB space on 4G vs ~2.5-3 s lazy — the doc
   comment in `install_revision` recording ~110 s for the old
   sequential eager copy remains the cautionary tale.

The branching-factor hypothesis is addressed in the dialog note:
**fanout 32 vs 256 produces identical trees today** — since dialog's
distribution rebalance, `max_segment` (64 KiB) sets block size, and the
soak shows 64 KiB is a good default (16 KiB is strictly worse; 256 KiB
buys little once bandwidth dominates). The leverage is not block size
but round-trip count and the per-request redeem.

## What changed in this audit

- **The join no longer stages at all.** The volatile staging pool, the
  install step (`install_claim_nodes` / `install_revision_between`), and
  the double-fetch they caused are deleted (~700 net lines). A join is
  now plain dialog against the durable replica: mount it hidden, save
  the candidate chain (the pull's forks prove with it — the remote
  honouring or refusing them is the authorization verdict), pull,
  validate, commit the roster claim, then index the replica visible.
  Atomicity needs no side store: an unindexed replica is invisible and
  resumable, content-addressed blocks are unobservable until a head
  references them, and a chain a failed attempt saved is inert
  authority that delegation pruning reclaims. Renewals become dialog's
  ordinary pull-merge instead of two full tree copies through the pool.
- **The first-push repair no longer full-scans.** `dialog-reactor`'s
  push fallback (which fires only on a typed missing-local-node failure;
  dialog's boundary-tolerant push diff handles the lazy case since
  dialog-db#454) now hydrates via `TreeDifference::compute` over the
  networked index — only the divergent paths the retried diff will
  visit — instead of streaming both entire trees one block per request.
  A test pins that a single-commit divergence over a 2 000-item base
  fetches a small fraction of the store.
- `claim_changes` (`router/join.rs`) now runs its three roster queries
  concurrently — one descent of latency instead of three.
- `join_invite` logs per-phase wall clock (`prepared`/`staged`/
  `committed`), so a slow join in the field is attributable at a glance;
  the staging phase is the network-bound one.
- dialog-db (pinned tag must be bumped to pick these up): tracing events
  on every remote UCAN effect (`redeem_ms` vs `storage_ms`,
  `permit_cache_hit`) and on every block hydration
  (`dialog::sync::hydrate`), plus the `dialog-soak` harness, the
  simulated-network `Fs` transport, and a nightly `soak:sync` regression
  gate.

## The join flow (for orientation)

`join_invite` = `prepare_join` (local: parse invite, resolve account,
claim the chain) → `stage_join` (network: mount a volatile replica,
pull, `validate_content`, `claim_changes`, commit the claim, validate
again) → `commit_join` (durable: mount the replica, install the claim's
novel nodes by tree diff — `install_claim_nodes` — reset the branch,
save authority, make the replica visible).

Renewals and local-only invites take `install_revision_between` instead:
a full snapshot export/import through the volatile pool — roughly two
full tree copies. Acceptable for local-only; worth revisiting for
renewals of large spaces.

## Remaining recommendations, ranked

1. **Batch or scope the permit redeem** (tonk-access-service +
   dialog-remote-ucan-s3). One redeem covering a subject's read space —
   a prefix-scoped presigned policy or a short-TTL bearer grant —
   instead of one per object. Measured ~2x on lazy-join latency at 4G;
   it also removes an ed25519 signature per block. This is the highest
   leverage change available.
2. **Reduce validation reads.** The `RepositoryName` select in
   `validate_content` is an unbounded concept query used as an
   existence probe; a point read on the subject's entity would touch
   one path. The second `validate_content` pass is warm and fine.
3. **Batch block reads at the dialog layer** (`GetMany`): collapses the
   16-wide fetch windows into single round trips. Pairs with (1).
4. **Sync drain fan-out** (`router/sync.rs`): repositories and branches
   sync strictly sequentially under one global lock. Fine for one
   space; a many-space account multiplies full round-trip chains.
   Bounded concurrency (2-4 repos) would cut multi-space sync time
   roughly proportionally.
5. **Progress signal.** The join holds the worker's write lock with a
   single pending→terminal status; a per-phase status (staging n/m)
   would make a slow join legible instead of indistinguishable from a
   hang.

## How to keep this honest (soaking)

dialog-db's `soak:sync` nightly arm replays the join scenario over
simulated networks (localhost/broadband/mobile/intercontinental),
gating round trips, bytes, and modeled time against a checked-in
baseline — see `rust/dialog-soak/README.md` there. When bumping tonk's
dialog pin, a red `soak:sync` on the dialog side is the early warning
that a join regression is about to ship here.

For tonk-level field data: the worker now logs join phase timings, and
dialog's `dialog::remote::ucan` events (behind a tracing subscriber)
attribute slow syncs to redeem vs storage vs descent without any
reproduction setup.
