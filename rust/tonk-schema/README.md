# tonk-schema

Typed schema for the facts stored on a repository's `meta` branch.

Every Tonk repository has a `meta` branch alongside its content branches. The
meta branch is a normal dialog-db branch — it syncs like any other — but its
artifacts describe the repository's *own* configuration: the replicas that hold
it, the branches it has, the remotes it tracks, and so on. This crate defines the
[`dialog_query::Concept`]s and [`dialog_query::Attribute`]s that make up that
schema, plus the analyzer-IR and wire types layered on top of it.

It is shared between the service worker (which reads and writes meta facts via
`dialog-reactor`), `slide`, and any client that needs to query the same shapes.
It sits above `tonk-core` and below `tonk-analyzer`/`tonk-evaluator` in the
dependency graph.

## Entity identity

Entities are identified as `did:key:z6Mk<base58>` URIs — the format dialog-db uses
everywhere. The base58 bytes come from one of two sources:

- **Intrinsic** — real cryptographic key material (profile DIDs, repository
  subject DIDs). The entity is whoever holds the keypair.
- **Content-derived** — the blake3 hash of a CBOR encoding of the entity's
  defining inputs. Two parties independently describing "the same thing" converge
  on the same entity, so the resulting artifacts merge cleanly when the meta
  branch syncs across devices. Import [`prelude::EntityExt`] and call
  `Entity::of(value)`.

The URI scheme is identical in both cases; the difference is in how the bytes are
produced, not how they're formatted.

## What's here

- **Meta-branch concepts** — the repository's self-description:
  [`Replica`](src/replica.rs), [`Branch`](src/branch.rs),
  [`Remote`](src/remote.rs), [`RepositoryName`](src/repository.rs),
  [`tracking_branch`](src/tracking_branch.rs).
- **User-schema definitions** — [`concept`](src/concept.rs) (user-defined
  concepts and the attributes naming their fields; concepts are identified
  structurally, by their field set) and [`resolution`](src/resolution.rs) (chain
  handles that reconstruct schema definitions from a source).
- **Rules** — [`rule`](src/rule.rs) (the rule-of-rules `Statement` adapter) and
  [`rule_query`](src/rule_query.rs) (surfacing installed rules as concept rows).
- **Built-ins** — [`builtin`](src/builtin.rs), the registry of concepts that are
  resolvable everywhere rather than stored as branch facts.
- **Wire / transact** — [`query`](src/query.rs) and
  [`query_source`](src/query_source.rs) (the on-the-wire `/query` body shape) and
  [`transact`](src/transact.rs) (the analyzer-IR types — `Application`,
  `Statement`, `Planner` — which live here because they reference schema-aware
  types like `rule::Rule`).
- **Sync state** — [`sync`](src/sync.rs), a pure, I/O-free classification of a
  branch's local head against its upstream.
- **Re-exports** — the wire-shape primitives `claim` / `conclusion` / `effect` /
  `meta` from [`tonk_core`].

## Conventions

The `#[derive(Concept)]` and `#[derive(Attribute)]` macros generate helper types
without doc comments, so the modules defining schema rows suppress `missing_docs`
locally. The crate otherwise builds under `#![warn(missing_docs)]`.
