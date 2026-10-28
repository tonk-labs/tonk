# Graph query integration

Companion to Dialog `ideas/graph-queries.md`, implemented on an isolated Tonk worktree.

## Contract

- Preserve ordinary concept descriptor encodings and flat query responses.
- Typed entity edges retain a concept bookmark, allowing self-reference without recursive descriptor hashes.
- Root matching and graph shaping are separate: shaping never removes a matched root.
- Return normalized nodes and per-node changes through existing evaluator/worker boundaries.
- Use Dialog's graph implementation and shared conformance checks, not a second evaluator.

## Checkpoints

1. Inspect declaration, persisted schema, evaluator and subscriptions; agree on Dialog API.
2. Declare and reload typed edges, including cyclic bookmarks, with analyzer/schema tests.
3. Evaluate a typed graph and expose it on the existing evaluate wire path with integrated tests.
4. Connect graph subscriptions and prove nested rename, edge movement and cyclic changes.
5. Run focused checks and fresh review; record exact evidence and unresolved limits here.

## Current evidence

- Tonk declarations are analyzed in `tonk-analyzer`, not the notation parser.
- Persisted fields use `db.concept.with/<field>` plus optional markers; graph target metadata must survive this reconstruction.
- `/evaluate` reexports `tonk-evaluator` match DTOs; `tonk-worker-api` has a wire-compatible copy.
- Dialog graph API coordination is in progress with the engine implementation agent.

## Implementation

- `7c7d0eb48f`: notation checkpoint; two focused declaration tests passed.
- The evaluator now resolves canonical graph shapes and runs graph reads against
  the transaction overlay, filters matching root rows, and serializes the graph
  alongside them. Ordinary responses omit the optional graph member.
- Query-side `graph.matching` and `graph.shaping` constants bind into Dialog scans.
- Existing worker `/query?graph=<bookmark>` supports one-shot graph reads and SSE
  `graph_snapshot` / `graph_delta` frames. Reactor sharing, shutdown, pending
  subscriber handshake, reconnect snapshots, and client cleanup are reused.
- Pinned schema edits clear obsolete edge metadata. Edge-target schema is captured
  when a subscription opens; reopening is required after schema changes.

## Verification

- `cargo check -p tonk-evaluator -p dialog-reactor` passed against the local Dialog
  graph branch.
- Native `cargo check -p tonk-worker` passed (existing dead-code warnings).
- Three end-to-end evaluator tests passed: cyclic normalized results and staged
  overlay updates; worker DTO JSON roundtrip; root typed conformance and separate
  matching/shaping semantics.
- Final focused run passed all eight tests: two declarations, five evaluator
  scenarios, and one reactor nested delta/reconnect/cleanup scenario.
- Existing analyzer, evaluator, and reactor unit suites passed in the fresh
  review cycle below.
- Local verification uses an external Cargo patch config. Its path and the
  resulting local-patch lockfile are not for committing. The dependency must be
  pinned to the reviewed Dialog graph commit before PR CI.
- Native HTTP/SSE handler requests and frames are verified below. Browser
  execution, live network serving, and CI are not yet verified here.

## Fresh review cycle

- Preserved ordinary fields named `graph` and their variable/constant bindings.
- Graph schemas now come from the analyzer's resolved scope. Recursive local
  redeclarations agree with their root predicates and before/after overlays.
- Concept resolution captures edge bookmarks in its existing metadata read;
  ordinary evaluation no longer re-resolves concepts or scans graph metadata.
- Field cleanup retracts only persisted graph metadata, preserving ordinary
  retraction statement counts. The existing one-retraction test passes unchanged.
- Graph fanout serializes a full snapshot only for pending subscribers, under
  the subscription-map lock after polling, preserving attachment during poll.
- Added explicit HTTP graph envelope with matching/shaping constraints and
  constrained subscription root changes, edge moves, cycle closure/cut tests.
- Full checkpoint: reactor 56 passed; analyzer 254 passed, 1 ignored; evaluator
  42 passed. Native HTTP/SSE handler test passed: constrained JSON equals the
  stream snapshot, nested change emits a root delta, invalid envelope rejected.
  Schema library suite: 135 passed. Formatting and whitespace checks passed.
- Review fixes committed as `98ddf4185a` (schema/analyzer compatibility) and
  `d74906c46a` (constrained streams/lazy snapshots). No test processes remain.

## Reviewed dependency pin

- Fresh Tonk review round two and Dialog review round three reported no
  actionable findings (parent handoff).
- All 18 Dialog workspace dependencies pin immutable revision
  `52f48d49ba952320a4c0025cf400ee1b9b277fe4`.
- Restored the committed lockfile before resolving this pin; no local-path patch
  state remains. All 25 resolved Dialog git packages use the same revision.
- `cargo metadata --locked --offline --format-version 1` and `git diff --check`
  passed. All new registry versions trace to the pinned Dialog dependency closure
  (S3 0.14, Iroh base 1.2, WASI 0.3); no broad update was run.
- No rebuild after pinning: preceding local tests exercised the graph source;
  remote-source build and CI verification remain with the parent PR workflow.

## CI follow-up on PR 1197

- Exact candidate `6815fa319a4b151ed04497d054208cc59f12ad24`, Test run
  `40685017199`: lint job `122805016492` fails two `collapsible_if` diagnostics.
  Scoped analyzer Clippy reproduced both (`graph.rs` and `query.rs`); collapsed
  the nested guards without changing conditions. Scoped Clippy with `-D warnings`
  now passes; formatting and whitespace checks pass.
- All four debug web shards and guest/browser builds fail duplicate Wasm exports
  from `wasm-streams` 0.5 (pinned Dialog storage) and 0.6 (Tonk plus Cloudflare
  worker 0.8.5). A Tonk-only downgrade cannot unify this dependency graph. Parent
  is coordinating the Dialog manifest fix and subsequent reviewed repin.
- Release-test success jobs only defer optimized testing to main/tags; they do
  not execute tests on this PR. Service-worker lifecycle tests did execute and
  passed. Account-test summary failure reports skipped prerequisites, not an
  executed account-test failure. CI is not green yet.
