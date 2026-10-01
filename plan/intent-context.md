# Intent: commands from what was typed and what is in view

Status: design, no code. The YAML is illustrative: it shows the shape and
has not been run through the notation. Follows
[command-palette-ubiquity.md](command-palette-ubiquity.md) and
[command-palette-resolver.md](command-palette-resolver.md).

## Goal

The system already defines commands that make things happen. The palette's
job is to **produce those commands' claims** from what you typed and what
is in view: the open notebook, the selected text, the time. Commands do not
change to suit the palette.

Example: on a notebook's page, "rename to Plans" produces
`notebook/retitle` with `subject` = the open notebook and `title` = "Plans".
With "Plans" selected, plain "rename" produces the same thing.

## The model

```
page                          worker                                 rules
────                          ──────                                 ─────
open palette / keystroke
  │ transacts
  ▼
intent/express ─────────────▶ parser (the command's handler)
{ expression, input,            │ writes to the session overlay:
  site, selection, time }       ▼
                              the expression (snapshot + input)
                              intent, one per command that fits ──▶ fragments on each intent
                              { expression, command, phrases }       (subject from the route,
                                                                      title from the selection
palette reads intents + fragments ◀──────────────────────────────────  or from typed text)
  → readings → Enter transacts the claim → keep a little memory
```

### `intent/express`: what the page sends

A command, transacted by the palette when it opens and on every change of
input. Its handler is the parser.

| Field | Type | Meaning |
| --- | --- | --- |
| `expression` | entity | One per opening of the palette, made by the page. Every `intent/express` from that opening names the same one. |
| `input` | text | Exactly what is typed now. |
| `site` | entity | The tab's site *on the branch being asked*: for a space, the space frame's site, which the worker stamps onto the space branch's overlay (`router/session.rs:327`). |
| `selection` | text, optional | The selected text, captured when the palette opens; absent when nothing is selected. |
| `time` | float | When the palette opened. |

### The expression and its intents: what the parser writes

The handler writes session-overlay facts: never stored, never synced, gone
with the tab. It writes them on the branch whose commands are being
proposed (the space branch for space commands, the profile branch for
profile ones).

- **The expression** (`this` = the `expression` entity): `input`, `site`,
  `selection`, `time`. The snapshot fields are written once, and `input`
  changes as the user types.
- **One `intent` per command the input could mean**, matched through
  `intent/action` names. Its id is derived from the expression and the
  command, so re-parsing updates it in place, and intents that no longer
  fit are retracted. Fields:
  - `expression`: the expression it came from;
  - `command`: the command it would produce;
  - phrases: the typed text the parser assigned to each role, e.g. goal →
    "Plans" in "rename to Plans".

The intent *is* the choice. Fragments for one command use that command's
own attributes, so several values for one field (e.g. `go <location>`,
one location per route) are several derived rows on one intent, not several
entities.

The parser stays code because rules can't split text into words. What it
produces is facts, so everything after it is rules.

### Fragments: what rules derive

A rule derives a value for one field of one command, onto an intent for
that command, using the command field's own attribute. Each field has a
one-field concept for this. The notation can generate it from the command's
fields.

For `notebook/retitle` (`notebook.yaml:401`), whose fields are
`xyz.tonk.notebook.retitle/subject` and `xyz.tonk.notebook.retitle/title`:

```yaml
rule!:
  description: The notebook this page shows is the one being renamed.
  assert: notebook/retitle-subject             # { subject: xyz.tonk.notebook.retitle/subject }
  where: { this: ?intent, subject: ?notebook }
  when:
    - assert: intent
      where: { this: ?intent, command: notebook/retitle, expression: ?expression }
    - assert: intent/expression
      where: { this: ?expression, site: ?site }
    - assert: site
      where: { this: ?site, concept: notebook/route }   # only notebook pages
    - assert: notebook/route                            # notebook.yaml:276
      where: { this: ?site, entity: ?notebook }

rule!:
  description: Selected text is a title candidate.
  assert: notebook/retitle-title               # { title: xyz.tonk.notebook.retitle/title }
  where: { this: ?intent, title: ?title }
  when:
    - assert: intent
      where: { this: ?intent, command: notebook/retitle, expression: ?expression }
    - assert: intent/expression
      where: { this: ?expression, selection: ?title }
```

Rules name the commands they serve (`command: notebook/retitle`), so a
rule only runs when its command is a live possibility.

Concepts match by shape, so `notebook/route` alone would also match a site
on another kind of page that carries an entity. The `concept` check on the
site is what says a notebook route actually matched.

### Assembling a claim

Dialog does not merge these fragments into the command by itself. A
concept's built-in rule reads stored attribute facts
(`concept/descriptor.rs:278`, and `DeductiveRule::from` at
`rule/deductive.rs:416`), so derived fragments are invisible to a query of
`notebook/retitle`. The palette therefore reads each field's fragments on
each intent and assembles readings itself. It has to anyway, because a
partial reading ("rename *this notebook* to …", with the title still
missing) is one where some fragments exist and others don't.

A reading with every field filled carries the claim. Enter transacts it as
a fresh command entity with those values.

### Actions: how commands are named

`intent/action { this: notebook/retitle, name: "rename" }` replaces
`lingo/verb`. A command can carry a shorthand that lowers to these facts:

```yaml
command!: &notebook/retitle
  action: [rename, retitle]
```

The fact stays the source of truth, because names can come from places
that do not own the command: a translation, a library adding a synonym, or
a user's own alias.

### Memory

Kept small:

- **which command was chosen for a typed prefix**, as `lingo/choice` does
  today (renamed under `intent/`);
- **the last value used per field**, for "the notebook you were just
  working on". To be decided when it is built.

The full expression is not kept.

### Lifecycle

| Moment | What happens |
| --- | --- |
| Open | The page makes an `expression` id and transacts `intent/express` with the snapshot. |
| Keystroke | `intent/express` again, with the new `input`. The parser updates the expression and its intents. Rules re-derive. |
| Enter | The chosen claim is transacted. A little memory is recorded. |
| Escape, or the tab goes | The overlay facts go. |

## Naming

The tonk side is `intent`: the `tonk-lingo` crate, the `lingo/suggest`
query (replaced by the above) and the `tonk.dialog.lingo.*` vocabulary all
move to it. `dialog-lingo` keeps its name: it parses language with no
dialog dependencies, and is not specific to this use. Nothing has shipped,
so renames are cheap now.

## Speed, and a fallback

Every keystroke becomes a transact, a parse, an overlay write, rule
evaluation and reads. Today it is one query. Ubiquity debounced input at
50 ms (`inputDelay`), which is a reasonable budget for the worker's part.

**Measure first:** keystroke to rows, in the worker, on a space with
realistic content, split by stage.

**If it is too slow, emulate the same model without the overlay.** The
query handler opens a transaction on the branch, stages the expression and
its intents, queries the fragments through `Transaction::query()` (which
reads staged writes; `dialog-repository`'s `transaction.rs:143`), and drops
the transaction without committing. The facts and rules are the same;
only where the facts live changes. So the rules written now keep working
when the overlay path is fast enough to replace it.

## Open questions

1. **Typed text versus the selection.** With "Plans" selected and "to
   Notes" typed, both are title fragments. Which wins: typed, then
   selection, then nothing? Ubiquity boosted the selection (×1.2), which
   suits the thing acted on but not a new value.
2. **Empty input.** If every rule requires `intent.command`, an empty
   palette derives nothing, and "what can I do with this open notebook?"
   has no answer. Either the parser proposes every command when the input
   is empty, or some rules don't require a command.
3. **Phrases.** How typed text per role is modelled, and whether the
   parser can produce several splits for one command.
4. **The route check.** How a library reference (`notebook/route`) compares
   with the value the site stamp writes in `site/concept`.
5. **Comparisons.** `text/length` exists in dialog. A `>` premise does not
   appear in its formula list. Recording the selection only when there is
   one avoids needing it.
6. **Confidence.** Scores order readings but don't say how sure the
   palette is. A margin between the top readings, and a higher bar for
   destructive commands, would decide when to fill without asking
   ([lessons](#what-to-learn-from-structured-decision-models)).

## First build

`notebook/retitle` end to end:

1. The `intent/express` command and its handler (the existing parser),
   writing the expression and intents.
2. The two rules above.
3. The palette reading fragments and assembling readings.
4. Tests: "rename to Plans" on a notebook page; plain "rename" with "Plans"
   selected; the same input on a page that isn't a notebook proposes no
   retitle.
5. The timing measurement, before building more on it.

# Background

The research behind the model.

## Mapping to tonk

What exists today, and where it would come from.

| Context | Ubiquity | tonk today | In this model |
| --- | --- | --- | --- |
| Where the user is | focused window/tab, URL | `site:<client>` in the profile's session overlay: `path`, `anchor`, `space` (text), `branch`, `branch-entity`, `replica`, `route`, `concept`, `profile-branch` | `intent/express`'s `site` |
| What the user is looking at | focused document | route params on the site: `site/entity`, `site/model`, `site/view` | read through `site` by rules (e.g. `notebook/route`) |
| What the user selected | selection text/HTML | **nothing** | `intent/express`'s `selection`, captured when the palette opens |
| What the user typed | input | the `input` term of `lingo/suggest` | `intent/express`'s `input` |
| Now | `new Date()` | the `now` term | `intent/express`'s `time` |
| Who is asking | logins | the profile; operator | not yet needed |
| Where output goes | focused element | a command handler's `site/request` on the tab (the bar acts) | out of scope here |
| Verb memory | `suggestion_memory` | `lingo/choice` facts on the profile | kept, renamed under `intent/` |
| Argument memory | (none; `CreateAdjective` by hand) | **nothing**: commands are transient concepts, so a run leaves no facts behind | open: a little is kept per run (see [Memory](#memory)) |
| Locale | parser language | none | `locale`, later |
| Collections (tabs, history, contacts) | noun types | concept queries on the branch | no change: rules read them like any other data |

The last row is the main departure from Ubiquity. A noun type mixed two
jobs: reading the world, and reading the context. Here the world is
already data, so only the context needs a home.

## What Ubiquity used as context

These are from the code at `mozilla/ubiquity@11dc94e`. Paths are relative to
`ubiquity/`.

| Source | What Ubiquity did with it |
| --- | --- |
| `context` object (`chrome/content/ubiquity.js:231-241`): `focusedWindow`, `focusedElement`, `chromeWindow`, `screenX/Y`; plus `menu` from the context menu | Passed to `preview`/`execute`. The parser stores it as `query.context` (`parser.js:1644`) and never reads it again. `screenX/Y` is never read. |
| Selection text (`contextutils.js:85-93`), or the link under the context menu | The only context the parser uses. It is interpolated as `object` into every parse (×1.2, `parser.js:991-1012`), substituted for the first anaphor (×1.2, `:1029-1047`), and allowed to move to other roles. On empty input it drives noun-first suggestions. |
| Selection HTML | Computed and never read by the parser. Commands read it through `CmdUtils.getHtmlSelection`. |
| Anaphora (`en.js:41`: this, that, it, selection, him, her, them) | Always resolved to the selection. Only the first match is replaced. |
| Focused element | Where output goes: `setSelection` writes into it (`contextutils.js:105-139`). |
| Current tab, URL, document | Command API. Also the `noun_type_url` default (0.5), and `activeElement.href` (0.7). |
| Open tabs, history, bookmarks, tags, search engines, extensions, commands | Noun types' `suggest`, each reading the browser directly. Tabs are invalidated on TabOpen/TabClose. |
| Contacts, Twitter users | Noun types reading stored logins. Cached forever. |
| Geolocation | `noun_type_geolocation` default and "here". Cached forever. |
| Now | Date/time defaults, and scoring that penalises dates other than today. |
| Locale, parser language | Which language parser (roles, delimiters, anaphora) is used. Also the Wikipedia language default. |
| Suggestion memory (`suggestion_memory.js`) | Verbs only, keyed by the typed verb prefix. Verb-first: `score^(1/(1+count))`; noun-first: `1-0.7/(1+count)`. |
| Command history (`cmdhistory.js`) | Recall only (Ctrl+Up/Down). No effect on scoring. |
| Prefs: `maxSuggestions`, disabled commands, `doNounFirstExternals` | Cut-offs and filters. |

Four things about it matter for us:

1. **Noun types never saw context.** `default()` is called with no arguments
   and cached until feeds reload (`parser.js:1393-1403`). So "the current
   URL" and "now" went stale between invocations, and default scores halved
   on every call (`:1408`). Noun types also never got the selection
   (`selectionIndices` is always null, `:1563-1568`).
2. **Context was scattered.** Every noun type read the browser its own way.
   There was no single place that said what the context is.
3. **Memory was verbs only.** Nothing remembered which *argument* was used.
4. **Previews and executes got the same `context`.** It was a live handle,
   not a description.

## What Ubiquity's commands used

The 88 commands Ubiquity subscribed by default: the built-in feed, plus
firefox, social, developer, pageedit, general, email, calendar, map and
search.

- **Most take little.** 33 take no argument, 45 take one, 8 take two and 2
  take three (`share on delicious`, `translate`). Of 67 argument slots, 52
  are `object`. `format` has 4, `source` 3, and `goal`, `instrument`,
  `alias` and `location` 2 each. `time` and `modifier` are never used. The
  role grammar mostly served two commands.
- **One argument type was shared: text.** `noun_arb_text` fills 32 of the
  67 slots. No other typed noun is used by more than two commands, and 19
  are used by one. Commands spanned unrelated services (tabs, Twitter,
  maps, Amazon), so there was little to share.
- **The current page was the main context.** About 36 commands read the
  current page, URL or window, and 11 read the selection in their own code.
  Every command with an argument got the selection through the parser as
  well.
- **Writing back was common.** 12 commands replace the selection with their
  result (`translate`, `calculate`, `link to Wikipedia`, `tinyurl`, …), and
  about 11 more change the page directly. "Transform what I selected" was a
  large share of real use.
- **Context defaults were scattered.** Some lived in noun types (current
  URL, today, geolocation, default search engine). Others were hard-coded
  in commands: `close tab`'s current tab, `translate`'s target language
  from prefs, Yelp's geolocation fallback.
- **No argument could depend on another.** A noun's `suggest` never sees
  the other arguments. `translate` swaps its languages, and Yelp searches
  near the geolocation instead of the given location, in execute code. The
  2009 "adjectives" request was asking for a real gap.
- **Composition was rare.** `CreateAlias` with fixed arguments (`anglicize`
  is `translate` to English) is the only command-to-command mechanism.
  `mixNouns` and a `__proto__` noun with a new `default` (weather: towns,
  defaulting to the geolocation) are the closest thing to "a shared type
  plus a context rule".

## What tonk's commands look like

The model above doesn't depend on commands sharing attributes. This
measurement is kept for if field reuse is considered later.

The 46 `command!` declarations in the library today:

- **Argument counts, not counting `time` nonces:** 8 take none, 18 take
  one, 11 take two, 7 take three, 2 take four. That's more arguments per
  command than Ubiquity had, and more multi-argument commands.
- **Field types:** 36 text, 32 entity, 11 float (mostly `time` nonces),
  5 integer.
- **No entity field is shared.** Every one of the 32 is a command-specific
  attribute (`xyz.tonk.pause-sync/space`, `xyz.tonk.command.check-update/space`, …).
  `tonk/rename-repository` has both a `subject` and a `space`.

Grouped by what they point at, they collapse to about 12 shared attributes:

| Points at | Fields | Commands |
| --- | --- | --- |
| space | 6 | rename (twice), replicate, remove, check-update, pause-sync |
| notebook | 5 | block insert/edit/place/remove, notebook retitle |
| block | 5 | block edit/place/remove, and insert's `prev` and `next` |
| sheet | 4 | rename-sheet, create-cell, create-column, create-row |
| cell, row, column, member, account, device, concept, … | 1–2 each | |

So sharing pays off much more in tonk than it did in Ubiquity. Tonk's
commands act on its own few kinds of thing, not on unrelated web services.
`block/insert` also gives a real same-branch dependency: its `prev` and
`next` must be blocks of the chosen `notebook`.

## What the 2009 posts asked for

From [Some mock-up around Ubiquity](https://github.com/Gozala/gozala.github.com/blob/main/deprecated.site/_posts/mozilla/ubiquity/2009-02-17-some-mock-up-around-ubiquity.md)
and [Adjectives](https://github.com/Gozala/gozala.github.com/blob/main/deprecated.site/_posts/mozilla/ubiquity/2009-03-26-adjectives-ubiquity-bugzilla-love.md):

- **Discourse memory.** An argument nobody gave defaults to what was used
  last, across commands and sessions: after `get 9876 in kde`, a bare `get`
  is still about KDE. Ubiquity had no such thing. `CreateAdjective` added it
  by hand per noun, with `history()`, `addHistory()` and a `memory` size.
- **Dependencies between nouns.** `BugById` depends on `Connection`: a
  candidate for one argument is only valid given another. A value can also
  make another argument unnecessary.
- **Ask only when there's no other way.** With one connection, it is used
  without asking.
- **Recency orders completion.** `8` completes to `846`, the most recent
  match.
- **Clipboard and screenshot** as candidates for `attach`.
- **Commands scoped by target** (`bugzilla get`, `bugzilla comment`).

## Prior art

From the docs and sources of Quicksilver, Alfred, LaunchBar, Raycast, VS
Code, Emacs (`interactive`, Embark, Marginalia, Consult), Apple App
Intents, PowerToys and the Ubiquity team's own writing. Links are inline.

### Context

Every system lands on the same few slots: the selection (typed as file,
text or URL), the current entity or document, the frontmost app or mode,
the clipboard, time, the last result, and the typed text.

- **Snapshot at invocation; don't resolve live.** Quicksilver resolves
  "Current Selection" live through proxies and caches them for 3 s. Its own
  docs describe the delay, and a "serious, well-known bug" with Finder
  selection
  ([Finder_Selection.md](https://github.com/quicksilver/Documentation/blob/main/Finder_Selection.md)).
  LaunchBar's Instant Send and Embark take the target once, when invoked.
  `intent/express`'s snapshot is captured once when the palette opens, and updated
  only by typing.
- **Applicability and the argument must be one fact.** VS Code's palette
  passes *no* arguments to a command. A `when` clause decides whether it
  shows, but "does not pass its context keys to the command handler", so
  handlers re-read the editor and can disagree with what made them
  applicable
  ([command guide](https://github.com/microsoft/vscode-docs/blob/main/api/extension-guides/command.md)).
  Here, the candidate that makes a command proposable is the value it runs
  with.
- **Ship a context inspector.** VS Code's "Inspect Context Keys" is how
  authors debug applicability. Rule authors here need "show this intent
  context and its candidates".

### Nouns, verbs, and which comes first

- **Users type nouns.** Ubiquity's parser author found that only 23 of 74
  built-in commands were really verbs, and users typed "weather" and
  "flickr" first
  ([DiCarlo, 2009](https://jonoscript.wordpress.com/2009/01/17/when-is-a-verb-not-a-verb/)).
  Raycast's most installed extensions (Chrome tabs, Spotify, Linear, Slack)
  are all "find a thing, then pick an action"
  ([store](https://www.raycast.com/store)).
- **Both directions are one mechanism.** Embark acts on the thing at point
  by inserting it into an ordinary command's first prompt. The remaining
  arguments are prompted for as usual
  ([README](https://github.com/oantolin/embark)). Alfred has keywords plus
  Universal Actions, and Raycast has root commands plus per-item action
  panels. Declare inputs on commands, and get the noun-first view by
  inverting the index.
- **Pure noun→verb→object was hard to learn.** Quicksilver's three panes
  are powerful, but reviewers describe them as feeling "a little weird"
  ([The Sweet Setup](https://thesweetsetup.com/apps/best-app-keyboard-launcher-mac/)).
  That is opinion, not a study.
- **Coarse types and a text escape hatch.** LaunchBar's actions accept only
  `string` and `path`. Alfred's Universal Actions know file, text and URL.
  Both sustained large ecosystems. Precise attributes help find candidates,
  but a strict match must never be the only way in.

### Arguments

- **Shared slots exist in shipped products.** In Raycast, "placeholders
  with matching names are replaced by the same value"
  ([dynamic placeholders](https://manual.raycast.com/dynamic-placeholders)):
  the name is the slot's identity.
- **A command needs every required argument filled or defaulted to be
  offered.** Apple drops an intent from Spotlight if its summary lacks a
  required parameter with no default
  ([WWDC25 #260](https://developer.apple.com/videos/play/wwdc2025/260/)).
  That is close to this design's rule, but Apple applies it to *showing*.
  We show with placeholders and apply it to *running*.
- **One argument's candidates can depend on another's.** Apple's
  `@IntentParameterDependency` filters one parameter's query by another's
  value
  ([doc](https://developer.apple.com/documentation/appintents/intentparameterdependency)).
  Ubiquity had nothing like it.
- **Keep argument lists short.** Raycast caps arguments at 3.

### Learning

- **Key it on (typed string, entity id).** Quicksilver adds `1 - 1/(n+1)`
  for the exact string typed for an object. Alfred "latches" the typed
  phrase to the chosen result, and uses a 4-week window
  ([result ordering](https://www.alfredapp.com/help/kb/understanding-result-ordering/)).
  Both need stable ids (Alfred's `uid`). Entities give us that.
- **Ranking actions per thing is harder than ranking things.** Quicksilver
  has per-type learned action ranking in its source, disabled
  (`QSExecutor.m`). Its actions are ranked by one global list.
- **Order recent, then similar, then everything.** That is VS Code's
  palette (`commandsQuickAccess.ts`). Alfred and PowerToys add fallbacks
  for text that matches nothing.
- **Nobody models "what we were discussing".** Quicksilver's result
  becoming the next subject, and Raycast's `launchContext`, last one hop.
  Emacs per-kind history is the closest thing. The argument-memory rule
  here would be new.

### Chaining

A result can become the next subject (Quicksilver), or be exported to
another view (Embark). In a fact database this is cheap: a command's result
is one more candidate in the context ("the thing just made").

## What to learn from structured-decision models

TypeSafe's Jev ([post](https://typesafe.ai/blog/introducing-system-one-models-and-jev),
[docs](https://docs.typesafe.ai/introduction)) is a hosted model, and we
are not using it or anything hosted. Its *interface* is worth learning
from, because it is a careful answer to the question the palette asks:
given a described situation, which of a known set of things is meant, and
how sure are we? Each lesson below applies to a local, deterministic
palette.

1. **The situation is one structured object.** Its `state` is a single JSON
   value holding everything relevant, kept apart from the questions asked
   about it ([State](https://docs.typesafe.ai/concepts/state)). That is
   the expression: facts about the situation in one place, and the
   schema (commands, attributes) as the questions.
2. **Decisions are closed, typed, described choice sets.** A `Choice` is
   one of named options, each with a description
   ([Choice](https://docs.typesafe.ai/primitives/choice)). For us that is
   which command (from the schema, matched against verbs *and* the
   command's `description`) and which value per attribute (from rules).
   Today's lingo resolves nouns inside the parser, so there is no such set.
   Closing the sets is what makes ranking a replaceable part.
3. **Every option gets a probability, and confidence is how peaked they
   are.** A flat distribution means unsure, one peak means sure. We can
   compute the same locally: normalise our scores over the options and
   look at the margin between the top two. Today we keep only the order.
4. **Act on confidence, with a threshold per action**
   ([confidence routing](https://docs.typesafe.ai/patterns/confidence-routing)).
   Fill an argument or run without asking only when sure, and require
   more for destructive commands: `expel member` needs more certainty than
   `view members`. That is a `stakes` fact on the command. It is the
   precise form of the 2009 "ask only when there is no other way".
5. **Ask every question at once, including speculative ones.** Their docs
   recommend asking about outcomes that only matter if another answer goes
   a certain way. For us: derive candidates for every attribute of every
   plausible command from one context, then choose, instead of parsing
   first and resolving nouns per reading.
6. **Always have a "none of these" option.** A choice without one forces a
   wrong answer. The palette needs an explicit outcome for "nothing fits":
   take the input as text, or say so.
7. **Large sets narrow in stages.** Above 255 options they choose
   level by level and keep the best few paths (beam search). For us: pick
   the kind of thing first (space, notebook, member), then the instance,
   rather than scoring every entity at once.
8. **Evaluate the decision function on a corpus.** Their "workflow evals"
   fix the workflow and compare answers on recorded inputs. For us: a
   table of `(context, input) → expected command and arguments`, run as a
   test, so ranking changes are measured rather than eyeballed.
