# Command palette in the FABB

Status: proposal. Nothing built.
Date: 2026-09-28

`Cmd+Shift+P` switches the FABB into a palette: a text field and a ranked list of
things you can do. Typing fuzzy-matches **verbs**, fills their roles with
**nouns** from the profile and the active space, and shows a **preview** of the
highlighted sentence before anything runs.

Verbs are not tonk commands. A command is a machine contract, shaped for its
handler. A verb is a way of saying something, shaped for a person, and it may
have no command behind it at all. The two can be reconciled by a rule where
that is useful, but nothing requires it.

This document proposes the model (verbs, nouns, roles, all as dialog
declarations), where the parser lives, where suggestions come from, and the
order of work. It supersedes the in-memory sketch on dialog-db's
`claude/nlp-parser-dialog-discovery-38UR9` branch, whose thesis this keeps and
whose mechanism it drops (see [Prior art](#prior-art)).

## What already exists

- Every `command!:` in `rust/tonk-core/assets/library/*.yaml` is a transient
  concept with a `description` and described fields (about 37 across `core`,
  `profile`, `notebook`, `table` and `prose`). The built-in `command` concept
  (`tonk-schema/src/builtin.rs`, `AnonymousConceptQuery::commands`) lists them per
  branch; `tonk query command` is that query.
- `event!:` maps one input source (a DOM event) onto a command's fields. It is
  the precedent for "a declaration that turns outside input into a transient
  concept".
- Views have a `label` facet (`show: label:`), for example `tonk:repository` →
  `{name}`. A noun suggestion is an entity rendered through that facet.
- Dialog concepts can have **concept-typed fields** (`conforms`, in
  `dialog-query/src/concept/descriptor.rs`). Nothing in tonk's YAML uses them
  yet.
- A dialog query can run against a **per-query overlay** (`.with(...)`,
  `notes/layered-rule-resolution.md`). Facts and rules in the overlay take part
  in the query without being committed.
- `tonk-prose/src-js/editor/fuzzy.ts` is a tested subsequence matcher.
  `heading-switcher.ts` is a quick-switcher UX. `tonk-language-server`'s
  `head_completions`/`field_completions` list concepts and fields with
  descriptions.

## Verbs are not commands

Ubiquity verbs have three parts: a grammar (names, plus arguments with roles
and noun types), a `preview`, and an optional `execute`. Many useful verbs
never execute anything ("define", "calc", "find"). Tonk commands have none of
the first two:

- **Most commands are not things a person says.** `block/place` takes a
  computed sequence `key`, `tonk/load` is stamped by `<tonk-site>`, and
  `block/insert` needs `next`/`prev` chain links.
- **Command fields are storage types, not argument types.**
  `space/remove.subject` is `as: entity`, which says nothing about which
  entities fit.
- **Commands cannot grow grammar.** Seeds are frozen at creation
  (`docs/evolving-command-concepts.md`), and changing a field's type changes the
  concept's identity. So metadata cannot be added *to* a command. A verb is a
  separate concept, and linking it to a command is a separate rule.

## The model

### A verb is a transient concept whose fields are roles

```yaml
verb!: &verb/rename
  description: Rename something
  names: [rename, retitle]
  with:
    object: { conforms: space }   # a role, typed by a concept
    goal:   { as: text }          # a role, typed by a value type
```

- **Fields are named by role** (`object`, `goal`, `source`, …), not by what a
  handler needs. That is the grammar.
- **A role's type is its noun.** `conforms: space` means candidates are
  `Query<space>`, displayed through `space`'s `label` view. `as: text` and the
  other value types are the built-in nouns. There is no separate `noun!`
  declaration: a noun type *is* a concept, and the verb holds the entity itself.
  Projecting it to what some command wants (a replica's `subject`, say) is the
  bridging rule's job, not the noun's.
- `names` are what fuzzy matching runs against. The first name is the display
  name.
- An optional `when:` is a query premise over the context bindings. It decides
  whether the verb applies at all. For example, "expel" applies only when the
  viewer's `MemberRole` is admin.
- A later `noun!` may add a *recognizer* (dates, URLs, amounts) for a value
  type. That is an extension, not part of the core.

A parsed sentence is a candidate **instance** of the verb concept: a set of
parameters, not yet asserted.

### Preview is a view on the verb

```yaml
view!:
  this: verb/rename
  show:
    preview: |
      rename <tonk-display entity={object} view=label></tonk-display> to “{goal}”
```

The highlighted candidate is placed in the per-query overlay, and the verb's
`preview` facet renders against it. Deductive rules can derive richer preview
content from the candidate, for example "these 14 blocks would be removed".
Nothing is committed. A verb with only a preview (a "find", "show" or
"define") is complete as it is.

### Execute is asserting the verb, and rules decide what that means

Running a suggestion asserts the verb instance as a transient, the way an
`event!` asserts a command today. What happens next is ordinary dialog:

- **A bridging rule to a command.** This is the reconciliation, and it is
  optional:

  ```yaml
  rule!:
    assert!: tonk/rename-repository
    when:
      - assert: verb/rename
        where: { object: ?space, goal: ?name }
      - space: { this: ?space, subject: ?subject }
    where: { space: ?subject, name: ?name }
  ```

- **A rule straight to durable facts**, as `prose/edit` does today, with no
  command in between.
- **A provider for the verb itself.** At commit time a verb is structurally a
  transient concept, so a `Provider<Verb>` works the same as one for a command.
  Navigation ("open X") belongs here: the handler posts a navigate to the
  originating client, per `commands-not-routes`.
- **Nothing**, for a preview-only verb, which then offers no "run".

This is also how the palette avoids offering a dead verb. A verb that is
neither preview-only nor read by any rule or provider would commit and do
nothing. Rules are facts (`dialog.rule/reads` records every attribute a rule
body reads), so "is anything listening to this verb?" is a query. Rust
providers are the gap: the worker's `CommandRegistry` is not visible as facts.
It should publish its registered concepts to the overlay so the same query
covers them.

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

- `{this}` is the site's entity, meaning what you are looking at. An empty role
  whose concept `{this}` conforms to is filled from it. That is Ubiquity's
  selection-as-argument, and it makes a bare "rename" mean "rename this".
- `{space}` and `{profile}` are the active space and profile. They scope
  candidate queries and `when:` premises.
- Verbs whose `object` accepts the current view's model rank up.

Today the FABB frame does **not** see the space-side site entity. It knows only
the `space` DID. Carrying `{this}` across is part of the key-relay work below.

## Where things come from

A palette session unions `verb`, `marker`, preview views and bridging rules
from three layers:

| Layer | Holds |
| --- | --- |
| **Built-in** (shipped with the binary, like `builtin.rs` concepts) | Tonk's own verbs: open, rename, share, invite, members, pause sync, new space, remove, and their bridges |
| **Profile branch** (`main@profile:tonk`) | Per-person verbs, aliases and markers |
| **Active space branch** | Verbs a space's author declared |

The built-in layer is not optional. A `verb!` added to `profile.yaml` next
month never reaches an existing profile, which is the frozen-seed failure
arriving through a new door. Built-in *concepts* already exist. Built-in
*rules*, meaning a bridge that fires at commit on a branch that never stored it,
are the open part (see [Open questions](#open-questions-and-a-harder-one)).

## The parser

**Recommendation: a pure Rust crate whose registry is dialog.** This is
Ubiquity's arrangement. Its parser was generic code, and everything it knew
about the language (verbs, their argument roles, noun types and their
suggestions, role markers) came from a registry that verbs were added to. Here
the parser hard-codes no verb, noun or marker. It runs against a
`Registry` built from a dialog subscription to `verb`, `marker` and the
concepts that roles conform to. Declaring a `verb!` on a branch is registering
it, and it becomes parseable and completable without touching the parser.

The registry is kept live: a subscription delta (a verb added in the space, a
new marker) updates it in place, so the palette never reloads.

What stays out of dialog is the string work. The old branch proposed parsing as
a rule cascade
(Token → VerbMatch → Segment → NounMatch → Candidate). Two things argue against
that:

- Dialog has no tokenizer and no fuzzy or scoring formulas. Its text formulas
  are `Like`/`Lowercase`/`Length`/`Concatenate`, plus a `StartsWith` constraint.
  Building subsequence scoring as datalog would be a research project that the
  palette waits on.
- Each keystroke would re-derive every candidate for work that is a few
  microseconds of string code. The overlay is worth its cost for one thing:
  the preview of the single highlighted candidate, debounced.

The split that does fit:

| Step | Where |
| --- | --- |
| Catalog of verbs and markers | dialog queries, three layers |
| Verb applicability (`when:`) | dialog query over the context bindings |
| Noun candidates | dialog `Query<concept>` with `StartsWith` pushdown, bounded |
| Tokenizing, marker segmentation, fuzzy scoring, ranking | Rust, pure |
| Preview of the highlighted candidate | dialog query and view render with the candidate in the per-query overlay |
| Running a suggestion | the verb asserted as a transient via `window.tonk.transact` |

The pipeline, per keystroke:

1. **Tokenize** and try each prefix against the registry's verb `names`
   (fuzzy, via a Rust port of `fuzzy.ts`).
2. **Segment** the remainder on the registry's markers into roles. Unmarked
   text goes to `object`.
3. **Recognize** each role's text against its noun, which the verb's field
   type names. Value types are recognized synchronously. A concept noun is
   completed by querying that concept, once per debounced change, and its
   candidates are shown through the concept's `label` view. Autocompletion is
   this step applied to the role under the cursor.
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
  `<input>`, `<tonk-menu>`/`<tonk-mi>` rows (each a verb name plus its filled
  roles rendered through their `label` views), and a preview area for the
  highlighted row.
- **The shortcut needs a relay.** Focus is usually inside the sealed
  `<tonk-site>` iframe, whose key events never reach the FABB's document. The
  guest must forward the chord over the portal bridge (`tonk-portal/src/bridge.rs`,
  `tonk-host/src/page_effect.rs`) along with its site entity, which gives the
  palette `{this}`. Nothing forwards keys across frames today.
- **The chord itself.** `Cmd+Shift+P` is Firefox's New Private Window, which
  pages likely cannot override. Verify in each browser before committing to it,
  and consider `Cmd+K` as the primary binding with `Cmd+Shift+P` as an alias.
- **Navigation is a verb too.** "Open X" over any routable entity resolves to
  the existing routes (`/{entity}@{model}`) through a verb provider, not a
  palette-local special case.

## Phases

Each phase ships something usable and tests one risky assumption.

0. **Palette over what the FABB already does.** It covers the FABB's existing
   actions (share, members, agent, pause, home) plus the space switcher's query,
   with fuzzy match, the key relay, and a mode in `BarState`. There is no
   schema change. This tests the relay, the chord and whether people use it at
   all.
1. **`verb!` as a transient concept, built-in layer.** Tonk's own verbs, with
   `object` as the only role and `text`, `space` and `member` as nouns.
   Execution goes through bridging rules or verb providers, which replaces the
   hard-coded list from phase 0. The "is anything listening" check lands here.
2. **Preview:** the `preview` facet and overlay rendering. Preview-only verbs
   become possible here.
3. **Roles, markers, `{this}`, noun-first** and context ranking.
4. **Author-declared verbs** on space branches. The CLI's `tonk do` and locale
   markers come here too.

## Open questions, and a harder one

- **Is free-form parsing worth it?** Ubiquity's parser was the hardest part of
  Ubiquity, and it is the part that did not survive. Most of a palette's value
  comes from fuzzy verb matching, contextual `{this}` and good noun suggestions,
  and phases 0–1 deliver all of that with prompted arguments. Before building
  phase 2, check whether people actually type "rename budget to q3" or pick
  "rename" and then fill a prompt. The model above supports both, but the parser
  is where the cost is.
- **Can tonk's YAML express `conforms`?** Dialog has concept-typed fields, but
  no tonk library uses them, and the analyzer and notation may not lower them.
  Check this before phase 1. If they can't, `noun!` comes back as a separate
  declaration naming the concept.
- **Where do built-in rules live?** A bridge rule must fire at commit on a branch
  that never stored it. Dialog's layered resolution reads rules from branches
  and the per-query overlay, not from a code-shipped layer. The options are
  seeding bridges on first palette use (which reintroduces freezing), teaching
  the reactor a built-in rule layer, or writing tonk's own bridges as Rust verb
  providers. The last needs no new mechanism.
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
  transient concepts. Those are the native home for "verb → effect", which its
  custom `install_verb`/`Effect` design was reaching for. Here a verb *is* a
  transient concept. This proposal keeps the vocabulary (roles, markers, noun
  types, scoring shape) and the idea that all of it is declared as facts. It
  drops parsing-as-rules.
- **Ubiquity** (Mozilla Labs, 2008–2009): verbs with preview and execute, noun
  types, Parser 2's semantic roles, and selection as an implicit argument.
- **VS Code palette:** a flat fuzzy list plus multi-step prompts. Phase 1
  deliberately stops here.
