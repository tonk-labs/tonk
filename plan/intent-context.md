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

Each run records the values it was given:

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

For each command whose verb matches what was typed:

1. Read its attributes.
2. For each one, take the candidates (from rules), or the typed text if the
   attribute is text.
3. If every attribute has a value, propose the reading with the
   best-weighted values. If exactly one candidate exists, use it without
   asking.
4. Otherwise, show the reading with the missing attribute as a placeholder.

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
2. **Dependency.** `expel member` takes `member/target`. Its candidates are
   the members of the tab's space, from that space's branch. A same-branch
   join is checked too, on a fixture where both values live on one branch.
   The cross-branch case (a member of a space named by an earlier argument)
   is expected to fail, and the test records how.
3. **Memory crosses commands.** Run `rename <space B> to X` from a tab on
   space A. A following `pause sync` ranks space B above the tab's space A
   only if the memory rule weighs recency above location. The test pins
   whichever order we decide. The point is that it needs no code per
   command.
4. **Ask only when needed.** With one candidate, the claim is complete.
   With two, the reading carries a placeholder and both candidates.

Each check reads facts with ordinary concept queries. No Rust is written
per command.

**What would falsify it:**

- **A command that needs a rule nobody else can use.** If every command
  ends up with its own candidate rule, shared attributes buy nothing over
  `lingo/argument`.
- **Gap 1 not closing.** If cross-branch context can't be expressed as
  overlay facts, context stops being data, and the palette is back to
  passing it in by hand.
