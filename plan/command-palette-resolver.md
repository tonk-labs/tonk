# Command palette: suggestions as a query

Status: investigation, no code. Follows [command-palette-sketch.md](command-palette-sketch.md).

## Goal

Everything through query and transact. The palette element subscribes to
suggestions for what was typed and renders the rows; running one transacts
its claim. No `window.tonk.palette`, no page-side command handling, no
second lane.

Today the element does dialog's work in JS: seven subscriptions per branch
(verbs, arguments, roles, nouns, attributes, choices), a descriptor lookup
and two more subscriptions per noun concept, then a synchronous call into
the parser.

## What dialog already offers

**Resolvers** (`dialog-query/src/resolver.rs`) are moded premises evaluated
through the environment. Their admission rule is the address space: a
resolver may read only content-addressed storage (`Load`), so its rows are
a pure function of its inputs. The `tree/*` family is the only one.
`ResolverQuery` is a closed enum.

**Subscriptions don't depend on resolvers for liveness.** Every `Select`
records the key ranges it read (the demand cover,
`dialog-repository/.../branch/subscription.rs`), and a write re-evaluates
the query only when it lands inside the cover. A premise that reads through
ordinary scans is therefore kept live for free. It falls back to full
recompute when it can't be restricted to one entity, which is right here.

**Any premise can run nested queries.** `Application::evaluate(selection,
env)` receives the recording `Scope`, so a premise that builds
`ConceptQuery`s and evaluates them against `env` has its reads recorded
like any other.

By dialog's own rule, then, a suggestion premise is **not a resolver**: it
reads mutable state through scans. That is fine, and simpler. It needs no
revision anchoring, because the demand cover already tracks what it read.

**tonk already routes virtual concepts.** `QueryPlan::from`
(`tonk-schema/src/concept.rs:1083`) sends a concept query whose predicate is
a sentinel descriptor to a tonk-side evaluator. That is how "concepts",
"commands" and "rules" are listed. The wire, the bridge, `subscribe` and the
reactor's subscriptions all go through `QueryPlan`, so a new sentinel needs
no new plumbing.

## Two paths

### A. A dialog premise (the destination)

`lingo/suggest(input, this, now) → {command, display, claim, score, …}`
as a premise in dialog-query, with `dialog-lingo` as its body.

It needs:

- **The palette schema in the dialog namespace.** The premise can't name
  `tonk.dialog.lingo.*` attributes.
- **Labels without tonk views.** Candidate labels come from the tonk view
  system (`xyz.tonk.view` `label` facet, rendered with tonk templates).
  dialog can't depend on that. Labels would come from rule-derived rows
  instead (e.g. `lingo/candidate {noun, label}` per noun concept).
- **An open or extended premise registry** in dialog-query.
- **tonk re-pinned on every dialog crate**, not just `dialog-lingo`.

It is right long term, and a large lift now.

### B. A tonk virtual concept (the shortcut)

Declare `lingo/suggestion` in the library as an ordinary concept. Its
fields are `input`, `this`, `now` (bound by the caller) and `command`,
`display`, `completion`, `claim`, `nouns`, `score` (produced).
`QueryPlan::from` routes it to a `SuggestionQuery` evaluator in tonk-schema
that:

1. evaluates the reads the element does today as `ConceptQuery`s against
   `env` (demand recorded, so subscriptions stay live);
2. builds the registry and parses with `dialog-lingo`;
3. yields one row per proposal (structured values as JSON text for now).

The element sends `{predicate: lingo/suggestion, terms: {input: "ren",
this: <space>, now: <ms>, command: ?, …}}` through `window.tonk.query` or
`subscribe`, renders the rows, and transacts `claim`. With `input: ""` it is
the menu.

Costs and catches:

- **Crate cycle.** `tonk-schema` would need `tonk-lingo`, which uses
  `tonk-template` for labels, and `tonk-template` depends on `tonk-schema`.
  The label renderer (`Segment`, `parse_segments`, `render_segments` in
  `tonk-template/src/lib.rs`) uses only `ipld`. It can move to a small crate
  that `tonk-template` re-exports, which breaks the cycle mechanically.
- **Latency.** Parsing moves from a synchronous in-page call to a worker
  round trip per keystroke. Inline completion needs a generation guard so a
  slow answer can't overwrite a newer one. The empty-state menu can stay a
  live subscription; typed input is better as one-shot queries.
- **Two branches.** A query is scoped to one branch. The element queries the
  space and the profile and merges the rows by score. Merging is small, but
  it is UI logic.
- **Memory across branches.** `lingo/choice` is on the profile, so the
  space's query can't read it. Either memory reinforces only profile verbs,
  or the element passes the counts in as an input.

B keeps every step inside query and transact, and it is the shape A would
have. Moving it into dialog later means moving the evaluator and the schema,
not the UI.

## Commands

Separate from the above, and needed either way: the FABB's actions as
regular commands whose outcomes are facts the FABB renders.

| Action | Command | Outcome |
| --- | --- | --- |
| go to tonk home | `tonk/home` (done) | navigation posted to the tab |
| copy share link | `tonk:invite` (exists) | `InviteState` rows `<tonk-share>` reads. The clipboard write must be opened in the gesture. |
| connect agent | `AgentHandoffRequest` (exists) | handoff state the agent panel reads |
| view members | new | view state as a fact on the tab's site entity (`xyz.tonk.site/panel`) |
| add an account | new | the ceremony, opened from a fact the same way |
