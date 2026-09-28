# Command palette in the FABB

Status: proposal. Nothing built.
Date: 2026-09-28

`Cmd+Shift+P` switches the FABB into a palette: a text field and a ranked list of
things you can do. Typing fuzzy-matches verbs, fills their arguments with nouns
from the profile and the active space, and running a suggestion asserts the same
transient command a button or a view would.

This document proposes the model (verbs, nouns, roles, all as dialog
declarations), where the parser lives, where suggestions come from, and the
order of work. It supersedes the in-memory sketch on dialog-db's
`claude/nlp-parser-dialog-discovery-38UR9` branch, whose thesis this keeps and
whose mechanism it drops (see [Prior art](#prior-art)).

## What already exists

More than the request assumes. **Commands are already registered, as data.**

- Every `command!:` in `rust/tonk-core/assets/library/*.yaml` is a transient
  concept with a `description` and typed `with:`/`maybe:` fields, each with its
  own `description`. There are about 37 across `core`, `profile`, `notebook`,
  `table` and `prose`.
- The built-in `command` concept (`tonk-schema/src/builtin.rs`,
  `AnonymousConceptQuery::commands`) already enumerates every transient concept on
  a branch with `name`, `description` and `source`. `tonk query command` is that
  query.
- A view's `bindings` (lowered in the analyzer) list every command the view
  can fire. That is a ready-made "commands relevant here" set for the active
  view.
- `event!:` is the precedent for everything below. An `event!` maps one input
  source (a DOM event) onto a command's fields, so the command stays
  domain-shaped and the input specifics live in a declaration beside it.
- Views already have a `label` facet (`show: label:`), for example
  `tonk:repository` → `{name}`. A noun suggestion is exactly an entity rendered
  through that facet.
- `tonk-prose/src-js/editor/fuzzy.ts` is a tested subsequence matcher.
  `heading-switcher.ts` is a quick-switcher UX. `tonk-language-server`'s
  `head_completions`/`field_completions` already list concepts and fields with
  descriptions.

So the palette does not need a command registry. It needs **a second input
source for commands, beside `event!`**.

## Why commands alone are not enough

Listing `command` rows in a picker would produce a bad palette, for three
reasons.

1. **Most commands are not things a person says.** `block/place` takes a computed
   sequence `key`, `tonk/load` is stamped by `<tonk-site>`, and `block/insert`
   needs `next`/`prev` chain links. A command is the machine contract, and it is
   shaped for its handler and its rule. A palette entry is a human affordance.
   `event!` exists because those are different things, and the same split
   applies here.
2. **Field types are storage types, not argument types.** `space/remove.subject`
   is `as: entity`. That says nothing about *which* entities fit, so there is
   nothing to suggest. The palette needs "an entity that conforms to `space`,
   contributing its `subject`".
3. **Commands cannot evolve.** Seeds are frozen at creation
   (`docs/evolving-command-concepts.md`), and marking a field optional or
   concept-typed changes the concept's identity. Adding palette metadata *to the
   command* would therefore fork every command from its seeded shape. Metadata
   *about* the command, keyed by it, does not.

## The model

Three declarations. All of them are ordinary concepts in the library, queried
like everything else.

### `verb!` — a way to say a command

```yaml
verb!: &verb/rename-space
  command: tonk/rename-repository
  description: Rename a space
  names: [rename, "rename space", retitle]
  roles:
    object: { field: space, noun: space }
    goal:   { field: name,  noun: text }
```

- `command` names the command it asserts. Several verbs may target one command,
  and a command with no verb is simply not in the palette. That is the opt-in,
  and it answers point 1.
- `names` are what fuzzy matching runs against. The first name is the display
  name.
- `roles` bind grammatical roles to command fields. A role names a **noun**
  (point 2). Fields the verb does not mention are filled the way `event!` fills
  them today, from a source expression:
  `time: ".timeStamp"` becomes `time: "{now}"`, and `subject: "{this}"` stays
  `{this}` (see [Context](#context)).
- Optional `when:` is a query premise over the context bindings, the same shape
  as a rule's `when`. It decides whether the verb applies at all. For example,
  `member/expel` applies only when the viewer's `MemberRole` is admin.
  Applicability is therefore a deductive query, not code in the palette.

### `noun!` — a kind of thing a role accepts

```yaml
noun!: &noun/space
  model: space        # the concept candidates must satisfy
  value: subject      # the field the command receives (default: this)
  # label: rendered through the model's `show: label` view facet
```

- **Candidates** are `Query<model>` on the relevant branch. The prefix of what
  was typed is pushed down with dialog's `StartsWith` where the label is a single
  attribute, and everything is fuzzy-ranked in Rust.
- **Display** is the model's `label` view. That is your "suggestion as a special
  view on a concept", and it already exists as a facet. A model without a
  `label` falls back to its first text field.
- **Built-in nouns** have no model: `text` (passthrough), `number`, `boolean`,
  `time` (natural dates later) and `any` (every entity with a routable model).

`noun!` could collapse into "a role names a concept directly". It is kept
separate because the value a command wants is often not the candidate's `this`:
`space` is keyed by the replica, while the commands want `subject`. It is also
separate because one concept may deserve several nouns (a `member` by name, or
by email).

### Roles and markers

The roles come from Ubiquity Parser 2: `object`, `goal`, `source`,
`location`, `instrument`, `time` and `modifier`. The markers that introduce
them are facts, per locale:

```yaml
marker!: { word: to,   role: goal,       locale: en }
marker!: { word: into, role: goal,       locale: en }
marker!: { word: from, role: source,     locale: en }
marker!: { word: with, role: instrument, locale: en }
```

Localization means asserting another set of markers. There is no code change.

### Context

The palette opens *somewhere*. That place is the `tonk:site` entity, which
`<tonk-site>` already stamps with `space`, `route`, `concept`, entity and view.
It becomes the implicit binding set:

- `{this}` is the site's entity, meaning what you are looking at. A role left
  empty whose noun accepts `{this}` is filled from it. That is Ubiquity's
  selection-as-argument, and it makes "rename" with no arguments mean "rename
  this".
- `{space}` and `{profile}` are the active space and profile. They scope
  candidate queries and `when:` premises.
- The view's `bindings` rank verbs up: commands the current view can already
  fire are the most likely ones.

Today the FABB frame does **not** see the space-side site entity. It knows only
the `space` DID. Carrying `{this}` across is part of the key-relay work below.

## Where things come from

A palette session unions `verb`/`noun`/`marker` rows from three layers:

| Layer | Holds | Dispatches to |
| --- | --- | --- |
| **Built-in** (shipped with the binary, like `builtin.rs` concepts) | Verbs for tonk's own commands: space create/rename/remove, invite, pause sync, members, home | The branch whose vocabulary handles it: profile or space |
| **Profile branch** (`main@profile:tonk`) | Per-person verbs and aliases | Profile |
| **Active space branch** | Verbs a space's author declared for their own `command!`s | That space |

The built-in layer is not optional. Frozen seeds mean a `verb!` added to
`profile.yaml` next month never reaches an existing profile, which is the
failure that doc describes, now arriving through a new door. Tonk's own verbs
therefore live in code-shipped descriptors, the way built-in concepts do, and
layered resolution makes a branch declaration shadow a built-in.

**Handled or not.** A verb over a rule-fulfilled command (table, notebook,
prose) works wherever its rule is on the branch. A verb over a Rust-provided
command works only if that branch's vocabulary registers it. `space_commands()`
is six commands, while `profile_commands()` is about 25. Without a check, the
palette would offer a command that commits and silently does nothing. That is
the exact silent-success class `evolving-command-concepts.md` records. Either
the worker publishes its registered command concepts as overlay facts that the
palette joins against, or built-in verbs carry their dispatch branch explicitly.
The first option is the honest one.

## The parser

**Recommendation: a pure Rust crate. The data lives in dialog, but the parse
does not.**

The old branch proposed parsing as a rule cascade
(Token → VerbMatch → Segment → NounMatch → Candidate). Two things argue against
that:

- Dialog has no tokenizer and no fuzzy or scoring formulas. Its text formulas
  are `Like`/`Lowercase`/`Length`/`Concatenate`, plus a `StartsWith` constraint.
  Building subsequence scoring as datalog would be a research project that the
  palette waits on.
- Each keystroke would become a transaction or overlay write followed by
  re-derivation, for work that is a few microseconds of string code.

The split that does fit:

| Step | Where |
| --- | --- |
| Catalog of verbs, nouns and markers | dialog queries, three layers |
| Verb applicability (`when:`) | dialog query over the context bindings |
| Noun candidates | dialog `Query<model>` with `StartsWith` pushdown, bounded |
| Tokenizing, marker segmentation, fuzzy scoring, ranking | Rust, pure |
| Running a suggestion | a transient claim via `window.tonk.transact`, as today |

The pipeline, per keystroke:

1. **Tokenize** and try each prefix as a verb name (fuzzy, via a Rust port of
   `fuzzy.ts`).
2. **Segment** the remainder on markers into roles. Unmarked text goes to
   `object`.
3. **Recognize** each role's text against its noun. Built-ins are synchronous.
   Concept nouns query once per debounced change.
4. **Noun-first:** if no verb matches well, treat the input as a noun and offer
   the verbs whose roles accept it. Typing a space name offers "open", "rename",
   "share" and so on.
5. **Score** lexicographically: verb match, then completeness, then noun
   confidence, then context boost. The old branch's
   `verb*1000 + completeness*100 + …` is a reasonable start.
6. **Incomplete verbs** are still selectable. Choosing one steps into its
   first empty required role, VS Code multi-step style, with that noun's
   candidates as the list.

The crate sits beside `tonk-notation`, target-agnostic and natively testable.
That also buys reuse the palette itself does not need: `tonk do "rename space
foo to bar"` in the CLI, and agents, use the same parser and the same verbs.

## The FABB side

- **Mode, not a new element.** `Panel::Palette` joins `BarState`. It shows an
  `<input>` plus `<tonk-menu>`/`<tonk-mi>` rows, and each row renders a verb
  name and its filled roles through their `label` views.
- **The shortcut needs a relay.** Focus is usually inside the sealed
  `<tonk-site>` iframe, whose key events never reach the FABB's document. The
  guest must forward the chord over the portal bridge (`tonk-portal/src/bridge.rs`,
  `tonk-host/src/page_effect.rs`) along with its site entity, which gives the
  palette `{this}`. Nothing forwards keys across frames today.
- **The chord itself.** `Cmd+Shift+P` is Firefox's New Private Window, which
  pages likely cannot override. Verify in each browser before committing to it,
  and consider `Cmd+K` as the primary binding with `Cmd+Shift+P` as an alias.
- **Navigation is a verb too.** "Open X" over `noun: any` resolves to the
  existing routes (`/{entity}@{model}`). Per `commands-not-routes`, it is a
  command whose handler posts a navigate to the originating client, not a
  palette-local special case.

## Phases

Each phase ships something usable and tests one risky assumption.

0. **Palette over what the FABB already does.** It covers the FABB's existing
   actions (share, members, agent, pause, home) plus the space switcher's query,
   with fuzzy match, the key relay, and a mode in `BarState`. There is no
   schema change. This tests the relay, the chord and whether people use it at
   all.
1. **`verb!` and `noun!` in a built-in layer**, for tonk's own commands, with
   `object` as the only role and `text`, `space` and `member` as nouns. The
   palette dispatches claims built from verb bindings, which replaces the
   hard-coded list from phase 0. The handled-or-not check lands here.
2. **Roles, markers, `{this}`, noun-first** and view-binding ranking.
3. **Author-declared verbs** on space branches, so a space's own `command!`
   becomes sayable. The CLI's `tonk do` and locale markers come here too.

## Open questions, and a harder one

- **Is free-form parsing worth it?** Ubiquity's parser was the hardest part of
  Ubiquity, and it is the part that did not survive. Most of a palette's value
  comes from fuzzy verb matching, contextual `{this}` and good noun suggestions,
  and phases 0–1 deliver all of that with prompted arguments. Before building
  phase 2, check whether people actually type "rename budget to q3" or pick
  "rename" and then fill a prompt. The model above supports both, but the parser
  is where the cost is.
- **Should `verb!` be mandatory for every `command!`?** An analyzer lint
  ("command with no verb and no event binding") would catch dead commands. It
  would also nag about plumbing commands, so it probably needs an explicit
  `internal: true`.
- **Scale of noun candidates.** `StartsWith` pushdown only helps when the label
  is one attribute. A noun over a large concept, with a label computed from
  several fields, needs a bounded scan or a derived search key. Measure this on
  a real space before designing around it.
- **Permissions versus applicability.** Should a `when:` premise hide a verb, or
  show it disabled with the reason? Hiding is simpler, but "why can't I expel?"
  then has no answer anywhere.

## Prior art

- **dialog-db `claude/nlp-parser-dialog-discovery-38UR9`** (Feb 2026,
  `rust/dialog-nlp`). This is an in-memory Ubiquity parser: `Verb`,
  `ArgumentSlot`, `SemanticRole`, `RoleMarker`, `NounType` with recognizers,
  lexicographic scoring, and 19 tests. It never touched dialog. Its dialog
  integration exists only in notes, and those predate inductive rules and
  transient concepts, which are the native home for "verb → effect" that its
  custom `install_verb`/`Effect` design was reaching for. This proposal keeps the
  vocabulary (roles, markers, noun types, scoring shape) and the idea that all of
  it is declared as facts. It drops parsing-as-rules.
- **Ubiquity** (Mozilla Labs, 2008–2009): verbs, noun types, Parser 2's
  semantic roles, and selection as an implicit argument.
- **VS Code palette:** a flat fuzzy list plus multi-step prompts. Phase 1
  deliberately stops here.
