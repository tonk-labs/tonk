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
