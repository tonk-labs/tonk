# Intent context: commands as attributes, arguments from context

Status: design, no code. The YAML below is illustrative: it shows the
shape, and has not been run through the notation. Follows [command-palette-ubiquity.md](command-palette-ubiquity.md)
and [command-palette-resolver.md](command-palette-resolver.md).

## The claim

A command is a combination of attributes. Commands reuse shared attributes
(`space/target`, `member/target`, …) instead of minting their own. Context
is data: an `intent/context` entity that describes where and when the
palette was asked. Rules derive candidate values for attributes from that
context. The palette proposes a command when every attribute it combines
has a candidate, or can take what was typed.

If that holds, then:

- `lingo/noun` and `lingo/argument` go away;
- "this is always the space" goes away;
- argument memory ("the bug we were just talking about") is one rule;
- the vocabulary left is verbs, synonyms, and which word introduces which
  attribute.

[Proof](#proof) says what would show the claim wrong.

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

## Mapping to tonk

What exists today, and where it would come from.

| Context | Ubiquity | tonk today | `intent/context` |
| --- | --- | --- | --- |
| Where the user is | focused window/tab, URL | `site:<client>` in the profile's session overlay: `path`, `anchor`, `space` (text), `branch`, `branch-entity`, `replica`, `route`, `concept`, `profile-branch` | `site` → the tab's site entity |
| What the user is looking at | focused document | route params on the site: `site/entity`, `site/model`, `site/view` | read through `site`; `focus` for an entity the view marks |
| What the user selected | selection text/HTML | **nothing** | `selection` (text) and `selected` (entities), written by the page when it opens the palette |
| What the user typed | input | the `input` term of `lingo/suggest` | `input` |
| Now | `new Date()` | the `now` term | `time` |
| Who is asking | logins | the profile; operator | `profile` (the profile entity) |
| Where output goes | focused element | a command handler's `site/request` on the tab (the bar acts) | out of scope here |
| Verb memory | `suggestion_memory` | `lingo/choice` facts on the profile | unchanged |
| Argument memory | (none; `CreateAdjective` by hand) | **nothing**: commands are transient concepts, so a run leaves no facts behind | `intent/used`: each run records the attribute values it was given |
| Locale | parser language | none | `locale`, later |
| Collections (tabs, history, contacts) | noun types | concept queries on the branch | no change: collections are candidates of a rule, not context |

The last row is the main departure from Ubiquity. A noun type mixed two
jobs: reading the world, and reading the context. Here the world is
already data, so only the context needs a home.

### The concept

```yaml
concept!: &intent/context
  this: tonk:intent/context
  description: Where, when and by whom the palette was asked.
  with:
    site:      { the: tonk.dialog.intent/site, as: entity }       # site:<client>
    input:     { the: tonk.dialog.intent/input, as: text }
    time:      { the: tonk.dialog.intent/time, as: float }
    selection: { the: tonk.dialog.intent/selection, as: text, optional: true }
    selected:  { the: tonk.dialog.intent/selected, as: entity, optional: true }  # many
```

It is written as session-overlay facts, like the site stamp. Nothing is
stored, and it goes when the tab does. One context entity per palette
session (`intent:<client>`), updated as the user types.

## How the claim manifests

### Commands combine shared attributes

Today every command mints its own attribute, so nothing can be said about
"a space argument" in general:

```yaml
# today
attribute!: &rename-space/space { the: xyz.tonk.rename-repository/space, as: entity }
attribute!: &pause-sync/space   { the: xyz.tonk.pause-sync/space,         as: entity }
```

With a shared attribute, both commands say "a space":

```yaml
attribute!: &space/target
  description: The space a command acts on.
  the: xyz.tonk.space/target
  as: entity

command!: &tonk/rename-repository
  with: { space: space/target, name: name/text }

command!: &tonk/pause-sync
  with: { space: space/target }
```

The attribute doubles as the role. "Move X to Y" is `move/from` and `move/to`,
two attributes. So lingo's roles reduce to which word introduces which
attribute ("to" → `name/text`), and that fact is shared too.

### Candidates come from rules over the context

A candidate is a value for a shared attribute, given a context. Dialog
rules conclude concepts, so a candidate is a small derived concept:

```yaml
concept!: &candidate/space
  with:
    context: { the: tonk.dialog.candidate/context, as: entity }
    space:   { the: xyz.tonk.space/target, as: entity }
    weight:  { the: tonk.dialog.candidate/weight, as: float }

# the space the asking tab is on
rule!:
  assert: candidate/space
  when:
    - assert: intent/context
      where: { this: ?context, site: ?site }
    - assert: site
      where: { this: ?site, space-entity: ?space }   # see "gaps"
  # weight 1.0

# any space this profile holds
rule!:
  assert: candidate/space
  when:
    - assert: intent/context
      where: { this: ?context }
    - assert: tonk/repository
      where: { this: ?space }
  # weight 0.3
```

One pair of rules serves rename, pause-sync, view-members and every later
command that takes `space/target`. The rule that says "the current space" is
what replaces the hard-coded `this`.

### Dependencies are joins, or branches

Where both values are on one branch, a dependency is a join: a rule that
reads one candidate to derive another. That is `BugById` depending on
`Connection`, without a `dependencies` array or a `reliable` getter. No
connection candidate means no bug candidate.

The case tonk has first is different. `member` (`core.yaml:806`) has no
space field: a space's roster lives on that space's own branch. So
"a member of *this* space" is not a join. It is the `member` rows of the
branch the candidate space names:

```yaml
# on a space branch: every member here is a candidate
rule!:
  assert: candidate/member
  when:
    - assert: intent/context
      where: { this: ?context }
    - assert: member
      where: { this: ?row, member: ?member }
```

The dependency on the space is carried by *which branch is asked*. That
works for the space the tab is on, since the palette already queries that
branch. It does not work for "a member of the space I mentioned two words
ago", which would need the palette to query a branch chosen by an earlier
argument. That is gap 1 again, in its sharpest form.

### Argument memory is a rule over what was used

Emacs keeps two things apart, and so should we:
- **past values** of an argument kind (`M-p`, one history list per kind);
- **likely values** from the context ("future history" on `M-n`: the file
  or URL at point).

It also keeps whole invocations with their arguments, so they can be
replayed (`command-history`). Here, likely values are the context rules
above. Past values are a record of each run and the values it was given:

```yaml
concept!: &intent/used
  with:
    attribute: { the: tonk.dialog.intent.used/attribute, as: text }
    value:     { the: tonk.dialog.intent.used/value, as: entity }
    time:      { the: tonk.dialog.intent.used/time, as: float }
```

and one rule per shared attribute turns that record into candidates, ranked
by recency. Because the attribute is shared, the space you renamed is a
candidate for the next `pause sync`. That is the "remembers whatever I'm
discussing" behaviour, and it needs nothing per command.

### What the palette does with it

There are two ways in, and they use the same index.

**Verb first.** For each command whose verb matches what was typed:

1. Read its attributes.
2. For each one, take the candidates (from rules), or the typed text if the
   attribute is text.
3. If every attribute has a value, propose the reading with the
   best-weighted values. If exactly one candidate exists, use it without
   asking.
4. Otherwise, show the reading with the missing attribute as a placeholder.
   A command is shown before its arguments are resolved, as a sentence with
   slots ("rename *space* to …"), and the rest are asked for in turn.

**Noun first.** When what was typed matches a thing rather than a verb, or
the context has a selected or focused entity: find which shared attributes
that entity can fill, and list the commands that take them. This is the
attribute → commands index inverted. Shared attributes give it for free;
per-command attributes can't. It matters because users type nouns
([Prior art](#prior-art)).

In both directions the candidate that makes a command proposable is the
value it runs with. There is no separate "is this applicable" check that
the handler then has to repeat.

Inapplicable commands are ranked lower, not hidden, and "take what was
typed as text" is always available. Hiding hurts discovery: Emacs leaves
mode filtering off in `M-x` by default for that reason.

The parser keeps what it is good at: splitting the input into verb and
delimited arguments, and scoring. It stops being where nouns are resolved.

## What it replaces

| Today | With this |
| --- | --- |
| `lingo/argument {command, field, role, noun}`, one per field | gone: the field's attribute is the role, and rules supply candidates |
| `lingo/noun` | gone: candidates are rule results |
| `lingo/role` | a word per shared attribute ("to" → `name/text`) |
| `lingo/verb` | stays: verbs and synonyms |
| the `this` term, always the space | `intent/context` plus rules |
| `noun()` in `lingo.rs`: every instance of a concept, plus a `label` facet | candidate rules; labels still from the `label` view facet |
| `role: lingo/now` time fields | still needed, and still a workaround (see gaps) |

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
  `intent/context` is written once when the palette opens, and updated
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
   `intent/context`: facts about the situation in one place, and the
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

## Gaps

These need resolving before or during the proof:

1. **Context lives on the profile; candidates often live on the space.**
   `site:<client>` is in the profile's session overlay. Members, documents
   and other per-space data are on the space branch. Dialog rules do not
   join across branches. The fix that keeps "context is data": write
   `intent:<client>` into the session overlay of *each* branch the palette
   queries. The palette already queries both and merges.
2. **`site/space` is text.** It is a repository name, not an entity, so a
   rule can't use it as `space/target`. The site stamp needs an
   entity-valued field for the space.
3. **Commands are transient.** A run leaves no facts, so argument memory
   needs `intent/used` to be written explicitly, as `lingo/choice` is today.
4. **Nothing records a selection.** The page would write it when it opens
   the palette. Without it, there is no anaphora and no noun-first.
5. **Weights.** Rules derive facts, not scores. A weight attribute on the
   candidate works, but two rules deriving the same value with different
   weights produce two rows. The palette takes the max. That is acceptable,
   but it is the palette's job, not a rule's.
6. **Re-fire timestamps.** `role: lingo/now` exists because a command with
   the same values would not fire twice. That is an engine question, not a
   language one. Keep it until the engine answers it.
7. **Migration.** Moving existing commands to shared attributes changes
   their shape. It is cheap for the transient bar commands, but any command
   whose claim another device reads has to move with care.

## Proof

Show it on two commands that should share an attribute, with rules and no
per-command glue. If per-command exceptions creep in, the claim is wrong.

A test in `tonk-worker` (next to `router::lingo`), on a fixture library:

1. **Shared attribute, one rule.** `rename-space` and `pause-sync` both take
   `space/target`. One context rule ("the tab's space") yields the same
   candidate for both, and the palette proposes both, filled, for empty
   input.
2. **Dependency.** Two cases:
   - **Same branch:** `block/insert` takes a `notebook` and `prev`/`next`
     blocks. With `block/target` shared, `prev` and `next` candidates come
     from a join rule over the chosen notebook's blocks. Without a notebook
     candidate, there are no block candidates.
   - **Across branches:** `expel member` takes `member/target`. Its
     candidates are the members of the tab's space, from that space's
     branch. A member of a space named by an earlier argument is expected
     to fail, and the test records how.
3. **Memory crosses commands.** Run `rename <space B> to X` from a tab on
   space A. A following `pause sync` ranks space B above the tab's space A
   only if the memory rule weighs recency above location. The test pins
   whichever order we decide. The point is that it needs no code per
   command.
4. **Ask only when needed.** With one candidate, the claim is complete.
   With two, the reading carries a placeholder and both candidates.
5. **Noun first.** With a notebook as the focused entity and no input, the
   proposals are the commands that take `notebook/target` (retitle, insert
   block, …), found by the inverted index and not by any per-command fact.

Each check reads facts with ordinary concept queries. No Rust is written
per command.

**What would falsify it:**

- **A command that needs a rule nobody else can use.** If every command
  ends up with its own candidate rule, shared attributes buy nothing over
  `lingo/argument`.
- **Gap 1 not closing.** If cross-branch context can't be expressed as
  overlay facts, context stops being data, and the palette is back to
  passing it in by hand.
