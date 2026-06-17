# tonk-core

Operation-type primitives: the claim, conclusion, effect, and meta wire shapes shared across the Tonk workspace.

This crate is the leaf at the bottom of the Tonk dependency graph
(`tonk-evaluator → tonk-analyzer → tonk-schema → tonk-core`). It holds the pure
operation-type cluster, the values that describe *what to do* to a repository and
*what came of it*. It depends only on `dialog-*` crates (`dialog-artifacts`,
`dialog-common`, `dialog-query`) plus serde/ipld plumbing, never on another
`tonk-*` crate, so these primitives can move into their own crate later.

## `claim`

The on-the-wire shape for `/transact` requests: typed assert/retract write-units
over a concept. A claim names a predicate ([`ConceptDescriptor`]) plus its
parameter bindings ([`PredicateApplication`]). The descriptor wrapper carries a
durability classification, `Durable` (facts carry forward across commits until
retracted) or `Transient` (facts live for one timestep, asserted so effects can
read them then retracted before the durable write), so the reactor can bucket
transients without re-querying the schema. Parameter bindings travel as a
`ValueMap` of concrete `dialog_artifacts::Value`s; the wire format has no
representation for logic variables or blanks.

## `conclusion`

The on-the-wire shape for query results: [`Conclusion`], a serializable
projection of dialog's `ConceptConclusion`. It carries the matched concept's
entity (`this`) and the projected field values keyed by query term name. Values
are encoded into the IPLD data model so the wire stays codec-agnostic (dag-json
on the browser hop, dag-cbor for storage). `Conclusion::project` reads variable
terms from the match bindings and emits constant terms directly, so filter
constants (`name = "Alice"`) still surface as fields.

## `effect`

The storage schema for effects: inductive rules with polarity. An [`Effect`]
pairs a compiled `dialog_query::InductiveRule` with a [`Polarity`]
(`Assert` produces persistent head facts, `Retract` produces retracts for the
matched cells). Effects are reified as facts on a branch (under the
`dialog.effect` domain) so they replicate and stay queryable like any other
concept, then loaded and fired on each commit. This module is the pure data type
and its storage-shape projections only; the loading/install-time query machinery
lives in `tonk_evaluator::effect_query`.

## `meta`

Cross-cutting metadata attributes in the `dialog.meta` and `dialog.name` domains,
scoped there (rather than under `xyz.tonk`) so facts written by Tonk and by other
dialog tooling name and describe the same entities mutually. Provides the [`Name`]
and [`Description`] concepts, the [`AnchorName`] newtype (a validated published
name that parses `id:<name>` into its [`Entity`] once at construction, erroring
hard on an invalid name rather than dropping the publication silently), and
[`AnonymousAttribute`] for describing attribute facts.
