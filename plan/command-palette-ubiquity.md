# How Ubiquity actually worked

Status: study notes. Companion to [command-palette.md](command-palette.md) and
[command-palette-sketch.md](command-palette-sketch.md).

Sources are the Ubiquity source at `github.com/mozilla/ubiquity`, commit
`11dc94e` (Jan 2011, the last). Paths below are relative to its `ubiquity/`
directory. Where this document says "the code does X", it was read, not
remembered. Design rationale from the wiki and blogs is in
[Documented intent](#documented-intent) at the end, and is kept separate from
what the code does.

## The parts

Parser 2 has five kinds of thing. Only two of them are written by command
authors.

| Part | Written by | Where |
| --- | --- | --- |
| **Language parser**: roles and their delimiters, branching, anaphora, word breaking, argument normalization | a localizer, once per language | `modules/parser/new/{en,ja,de,ca,$,…}.js` |
| **Noun type**: `suggest`, `default`, `label` and some flags | command authors, or built in | `modules/nountypes.js`, `modules/nounutils.js` |
| **Command**: `names`, `arguments` as (role, nountype), `preview`, `execute` | command authors | `modules/cmdutils.js` `CreateCommand` |
| **Command localization**: translated names, descriptions and preview strings | a localizer, per command | `localization/<lang>/*.po` |
| **Suggestion memory**: counts of (typed verb text → chosen verb) | the user, by using it | `modules/suggestion_memory.js` |

The load-bearing split is between the first and third rows. **A command never
names a connecting word.** It says "my second argument is a `goal` of type
language". The English parser says `goal` is introduced by "to", and the
Japanese parser says `goal` is marked by the postposition に. Every command
therefore parses in every language that has a parser, and translating a
command means translating its names and strings, never its grammar.

## Language parser

`en.js` is the whole English grammar:

```js
new Parser({
  lang: "en",
  anaphora: ["this", "that", "it", "selection", "him", "her", "them"],
  roles: [
    {role: "goal", delimiter: "to"},
    {role: "source", delimiter: "from"},
    {role: "location", delimiter: "near"}, {role: "location", delimiter: "on"},
    {role: "location", delimiter: "at"},   {role: "location", delimiter: "in"},
    {role: "time", delimiter: "at"},       {role: "time", delimiter: "on"},
    {role: "instrument", delimiter: "with"}, {role: "instrument", delimiter: "using"},
    {role: "format", delimiter: "in"},
    {role: "modifier", delimiter: "of"},   {role: "modifier", delimiter: "for"},
    {role: "alias", delimiter: "as"},      {role: "alias", delimiter: "named"}
  ],
  branching: "right",
  verbFinalMultiplier: 0.3
});
```

- **The roles** are `object` (implicit, usually no delimiter), `goal`,
  `source`, `location`, `time`, `instrument`, `format`, `modifier` and
  `alias`.
- **One delimiter can mean several roles** ("at" is `location` or `time`, and
  "in" is `location` or `format`). The parser tries each, and the command's
  argument list and the noun types settle it.
- **`branching`** says which side of the delimiter the argument sits on.
  English is `"right"` ("to *bob*"). Japanese is `"left"` ("*ボブ*に").
- **Verb position.** `suggestedVerbOrder` is 0 for verb-initial and −1 for
  verb-final. German makes it a function of the verb name. Each side has a
  multiplier: English puts 0.3 on verb-final parses, and Japanese puts 0.3 on
  verb-initial ones.
- **`usespaces` and `joindelimiter`.** Japanese has no spaces, so
  `ja.wordBreaker` inserts U+200B zero-width spaces around every particle
  before splitting.
- **`normalizeArgument`** strips articles. Catalan returns
  `{prefix: "el ", newInput: "google", suffix: ""}`, and the parser keeps both
  the raw and the stripped parse.
- **`$.js`** is a language made of symbols (`>` goal, `<` source, `@`
  location, `+` instrument, `=` alias, `%` format, `*` modifier). It is a
  power-user syntax, and it falls out of the same machinery for free.

Command names are localized through `.po` files. The Japanese "bold" has 12
names, because it enumerates conjugations (太字にする, 太字にして, 太字にしろ, …)
rather than doing morphology.

## Noun types

A noun type is an object with a `suggest` method:

```js
{
  label: "language",                                   // shown for an unfilled argument
  suggest(text, html, callback, selectionIndices) {    // sync return and/or async callback
    return [CmdUtils.makeSugg(text, html, data, score, selectionIndices)];
  },
  default: sugg | [suggs] | function,                  // used when the role is unfilled
  noExternalCalls: true,                               // local; allowed in noun-first
  cacheTime: -1 | 0 | seconds,                         // -1 forever, 0 never
  registerCacheObserver(flush) { … },                  // flush cached suggestions on change
}
```

- **A suggestion** is `{text, html, data, summary, score}` (`makeSugg`).
  `data` is the resolved value (a language code, a tab object, a search
  engine). `summary` is text truncated to 35 characters, with the part that
  came from the selection wrapped in a span. Score defaults to 1.
- **Scores are the noun type's confidence** that the text is one of its kind.
  `noun_arb_text` returns 0.3 for anything. `noun_type_email` returns 1, or 0.8
  when the domain's TLD looks wrong. A partial string match uses `matchScore`:

  ```
  0.3 + 0.25·√(matchedLength / inputLength) + 0.45·(1 − matchIndex / inputLength)
  ```

  So an early, long match scores high. The tests (`testVariableNounWeights`)
  check that a noun type returning score 2.0 ranks its verb above one returning
  1.0. Scores are not capped at 1.
- **Constructors** (`NounUtils.NounType`) cover the common cases:
  - an array of words is a closed set;
  - an object maps each text to a `data` value (`noun_type_lang_google` is
    `{Afrikaans: "af", …}`);
  - a `RegExp` is a recognizer, whose `data` is the match object.

  `mixNouns(a, b, …)` is a union: it concatenates their suggestions.
- **Async.** `suggest` may return request objects (XHRs) alongside
  immediate suggestions, and call `callback(suggs)` later.
  `noun_type_contact` returns the email recognizer's guesses at once and then
  Gmail contacts through the callback. The parser tracks outstanding requests
  by `readyState`, and cancels them when the input changes.
- **`registerCacheObserver`** is how `noun_type_tab` flushes its cached
  suggestions on `TabOpen` and `TabClose`. It is a subscription in all but
  name.
- **Dead flags.** `rankLast` and `noSelection` are set on built-in noun types
  but read only by `parser/original/parser.js`, which is Parser 1. Parser 2
  ignores them. The effect `rankLast` was meant to have comes from
  `noun_arb_text`'s low 0.3 score instead.

## Commands

```js
CmdUtils.CreateCommand({
  names: ["translate"],
  arguments: [
    {role: "object", nountype: noun_arb_text},
    {role: "source", nountype: noun_type_lang_google},
    {role: "goal",   nountype: noun_type_lang_google}],
  description: "Translates from one language to another.",
  help: "…try issuing \"translate mother from english to chinese\"…",
  preview(pblock, {object, source, goal}) { … },
  execute({object, source, goal}) { … },
});
```

(`standard-feeds/general.js`)

- **`arguments`** is a list of `{role, nountype, label?, default?}`, at most
  one per role. There are shorthands: `{object: noun}`, `{"object label":
  noun}`, a bare noun for `argument`, and an array or dict that becomes a
  noun type. Each role's value reaches `execute` as the *first* suggestion
  object, `{text, html, data, summary, score, input}`.
- **`names`.** The first is the canonical name and the id. Every name, and
  every word-suffix of a multi-word name, is a match target: "cogit ergo sum"
  also matches from "ergo sum" and "sum".
- **`preview(pblock, args)`** writes HTML into a live preview element. It runs
  after `previewDelay` ms (default 150) of no typing, so a preview that makes
  a network call does not fire per keystroke. `previewUrl` loads a page first
  and hands its `<body>` as `pblock`. The default preview is the description.
- **`execute(args)`** does the thing. A string `execute` that parses as a URL
  opens it.
- **`CreateAlias({names, verb, givenArgs})`** makes a new command that is
  another one with some roles fixed. `anglicize` is `translate` with
  `givenArgs: {goal: "English"}`. The given text is run through the target's
  noun type as if typed, and the role is `hidden` from display.
- **Composition.** `germanize` shows the other form: a command whose preview
  and execute call `CmdUtils.previewCommand("translate", …)` and
  `executeCommand("translate", …)` with the goal fixed.

## The pipeline

`ParseQuery#_yieldingParse` (`parser/new/parser.js` ~L1784) runs these steps as
a generator. It yields to the event loop between steps so typing stays
responsive, and a new keystroke cancels the query.

1. **Word break.** This is the language hook: Japanese inserts zero-width
   spaces around particles.
2. **Verb finding** (`verbFinder`). The input yields a trivial "no verb" pair,
   plus a pair for every verb name prefix-matched at the **start** (verb-
   initial) or the **end** (verb-final) of the input. The score is:

   ```
   verbScore = (0.4 + 0.6·√(typedPrefixLength / nameLength)) · orderMultiplier
   verbScore = verbScore ^ (1 / (1 + memory(typedPrefix, verb)))
   ```

   The 0.4 floor keeps a one-letter prefix above noun-first suggestions (0.3,
   trac #750). The memory boost takes the n-th root of a score in [0, 1]: each
   past choice of *this verb for this exact typed prefix* pulls the score
   toward 1.
3. **Clitics.** This step is not implemented (`// TODO: find clitics`).
4. **Argument finding** (`argFinder`), the core of the parser:
   - It finds every word that could be a delimiter of the verb's roles.
     Without a verb, that is every role's delimiter.
   - It takes the **power set** of those positions, skipping adjacent
     delimiters.
   - For each set, it assigns every possible span on the branching side of each
     delimiter to each role that delimiter can mean. Leftover words become
     `object`.
   - `DONT_PARSE_MULTIPLE_ARGS_PER_ROLE` prunes parses with two arguments in one
     role (except `object`), for speed.

   So "email hello to jono" yields `{object: "hello to jono"}` and
   `{object: "hello", goal: "jono"}`, among others. Ambiguity is enumerated,
   not avoided.

   **Selection interpolation.** If text is selected, every parse also gets a
   copy with the whole selection added as an `object` argument, and its score
   multiplier is ×1.2.
5. **Anaphora.** If there is a selection and the input contains an anaphor
   ("this", "it", …), each argument containing one gets a copy with the
   selection substituted, ×1.2.
6. **Normalization.** This runs the language's `normalizeArgument`
   (strip articles), keeping the stripped part as `inactivePrefix`/`Suffix` for
   display.
7. **Objects to other roles** (`applyObjectsToOtherRoles`). An unmarked object
   is also tried in every other role, using that role's first delimiter, so
   "calendar" can be read as "add *to* calendar" and "google" as "search *with*
   google". It is skipped when the verb is known, takes an object, and the
   object was typed rather than selected: "twitter hello" must not become
   "twitter as hello". Parses whose set of roles no verb could accept (checked
   against the power set of every verb's roles) are dropped.
8. **Verb suggestion** (`suggestVerb`), the **noun-first** path. A parse with
   no verb gets one copy per verb whose arguments cover every role in the
   parse.
   - Its multiplier is 0.3, and its verb score is `1 − 0.7 / (1 + timesUsed)`,
     which ranks by usage alone.
   - Only noun types with `noExternalCalls` count, unless a preference allows
     it, so noun-first never fires a network request per verb.
9. **Noun detection.** Each distinct (argument text, noun type) pair is
   detected **once per query and cached across queries**. The cache has a TTL
   of `cacheTime`, a default of one day, and −1 means forever. It is keyed by
   text, so "din", "dine" and "diner" are separate entries.
10. **Argument suggestion** (`suggestArgs`). A parse becomes one parse per
    **combination** of its roles' suggestions (a cartesian product). A role the
    noun type returns nothing for kills the parse. Unfilled roles are then
    filled from, in order:
    - the argument's `input` (aliases);
    - the argument's `default`;
    - the noun type's `default`;
    - an empty suggestion.

    Every default's score is halved. In a language whose object role has a
    delimiter (Japanese を), an object *without* it is ×0.6.
11. **Score and rank.** With `m` = score multiplier (verb score × 0.3 if
    suggested × 1.2 per selection use × 0.5 per extra argument in a role):

    ```
    score = m + Σ over filled roles ( firstSuggestion.score · m )
    ```

    So a verb with more well-matched arguments outranks one with fewer, and
    everything scales with how sure the verb match was.

**"Rising Sun" pruning** (`addIfGoodEnough`) keeps only parses whose `maxScore`
could still beat the lowest of the top `maxSuggestions`. `maxScore` assumes an
async argument still pending will score 1. Results render at 80% detection
progress or when async results land, and don't wait for the slowest noun type.

The first suggestion is highlighted and previewed. Enter runs
`strengthenMemory` (records (typed verb prefix → verb) and ("" → verb)), then
`execute`. If Enter arrives before any results, the execute is queued and runs
on the first result. With no input and a selection, the context menu runs the
same query, which is noun-first suggestion over the selection.

## Suggestion memory

The memory is one SQLite table, `(id_string, input, suggestion, score)`, with
the parser's memory under `"main_parser"`. Only **verbs** are remembered, keyed
by the typed verb prefix plus a global "" row. Noun choices are never learned.
Picking "Alice" over "Alicia" teaches nothing.

## What the tests show

`tests/test_parser2.js` doubles as a specification, and it also shows where the
design was not settled:

- `testSortSpecificNounsBeforeArbTextParser2`: with "beagle" selected and no
  input, "wash" (a dog noun) ranks above "mumble" (arbitrary text).
- `testImplicitPronounParser2` carries the authors' own `TODO FAILURE`
  notes. With selection "dine", "eat lunch at selection" also yielded "eat
  dinner at diner" and "eat near diner at diner" ("bizzare", per the comment).
  Selection interpolation plus role re-assignment over-generates.
- `testVerbUsesDefaultIfNoArgProvidedParser2` and `testNounsWithMultipleDefaults`
  show that defaults fan out: one parse per default value.

## What this means for us

These are the facts that constrain a faithful port. They are sourced from the
code above and are not yet design decisions.

1. **Grammar belongs to the language, arguments belong to the command.** A
   tonk port declares role → delimiter once per locale. A command declares only
   which of its fields play which role, and with what noun type.
2. **A noun type is suggest-with-confidence, not a lookup.** It returns
   several interpretations, each scored, and `data` is the real value. For a
   dialog concept, `data` is the entity and `text`/`html` are its label.
3. **The parse is generate-and-score, not deterministic.** Power sets of
   delimiter positions, role re-assignment, selection interpolation and
   cartesian products of suggestions all over-generate on purpose. Noun
   confidence and verb confidence then pick. The cost is controlled by
   caching per (text, noun type), pruning with `maxScore`, and yielding.
4. **The selection is the most important argument source.** It is
   interpolated into every parse (×1.2), substituted for anaphora (×1.2), used
   for noun-first suggestions, and powers the no-input context menu.
5. **Noun detection is the only expensive, async part**, and the cache is keyed
   by (text, noun type) with an explicit invalidation hook. For us, dialog
   subscriptions are that invalidation hook.
6. **Memory is shallow**: verb counts per typed prefix. It is a cheap place to
   do better (noun memory, context-conditioned memory) without changing the
   parser.

## Documented intent

From the Mozilla wiki (raw pages, including old revisions), mitcho's blog and
his SIGIR 2009 paper, and Jono DiCarlo's blog. Aza Raskin's own posts are gone
(azarask.in is squatted), so his position is known only through quotation.
There is **no** wiki page for Parser 2 scoring, memory or noun types. That
material lives in blog posts and the Parser 1 docs.

### What it was for

- **Efficiency, not natural language.** From the Roadmap: *"Natural language
  and generativity are of interest mainly to developers. For the end-user, the
  benefit… is that it gives a faster way to do common web tasks."*
  ([Roadmap](https://wiki.mozilla.org/Labs/Ubiquity/Roadmap))
- **Jono's riddle:** *"How can we make a UI with the efficiency and
  expressiveness of the Unix command line, but that's easy to learn and that
  won't shoot you in the foot?"* His requirements map one-to-one onto
  features:
  - preview;
  - suggestions for the selected data type (noun-first);
  - *"start with the noun or… the verb"*;
  - memory of past choices;
  - the selection as *"input for any of the multiple arguments — or for none of
    them"*;
  - ambiguity resolved by the user.

  ([part 1](https://jonoscript.wordpress.com/2008/07/21/language-based-interfaces-part-1-the-problem/))
- **Noun-first corrects Enso.** Enso, the predecessor, was strictly verb-noun
  with one argument. Its users' top requests were abbreviations and
  noun-first. Jono: *"I'm now convinced the users were right."*
- **Natural syntax** (mitcho): *"The grammar must never conflict with a user's
  natural intuitions about their own language's syntax."* The lexicon may be
  restricted, but the syntax must not be. Input is deliberately limited to *"a
  single verb and its arguments"*.
  ([paper](https://mitcho.com/research/ubiquity.pdf),
  [post](https://mitcho.com/blog/projects/how-natural-should-a-natural-interface-be/))
- **Ambiguity is shown, not guessed.** From the paper: *"a list of possible
  parses is presented to the user for confirmation before execution."*

### Why roles

- **Parser 1 had one English preposition per argument.** That broke on
  case-marking languages and synonymous adpositions, and it *"requires command
  authors to make localized versions of their commands."* With roles, `move
  {object, source, goal}` works as *"move truck from Paris to London"* and as
  *"truckをParisからLondonへmove"*. *"All that remains to be localized… is the
  name of your verb."*
  ([post](https://mitcho.com/blog/projects/writing-commands-with-semantic-roles/))
- **Roles encode syntax, noun types encode content.** Roles *"should map to
  morphological features in languages, not necessarily to the type of content
  in the argument (which is why we also will keep the noun types)."* Time and
  location share markers, so the noun type must disambiguate.
  ([Semantic Roles](https://wiki.mozilla.org/Labs/Ubiquity/Parser_2/Semantic_Roles))
- **The test for splitting a role:** *"would these markers translate to the
  same markers in a different language?"* "With Jono" (と) and "with Google"
  (で) are different roles.
- **Principles and parameters.** One universal parser plus a language file of
  *"ten to thirty lines"*. Localization *"reduced to little more than some native
  speaker consultation and string translation."*
- **Delimiters, not every substring,** because otherwise noun detection would
  run on every substring of the input.
  ([post](https://mitcho.com/blog/projects/in-case-of-case/))
- **Verbs only initial or final,** from a typology survey of imperatives.
  ([post](https://mitcho.com/blog/observation/wheres-the-verb/))

### Scoring rationale

- **Parser 1 ranked lexicographically, in Optimality Theory style:** memory
  first, then verb match, then argument match. Synonyms scored below names *"to
  prevent synonyms from colonizing the namespace."*
  ([post](https://mitcho.com/blog/observation/scoring-and-ranking-suggestions/))
- **Parser 2's additive form with a multiplier,** where all penalties go into the
  multiplier early, exists so that the score only rises as detection completes.
  That makes `maxScore` a sound upper bound, so async noun detection can be
  skipped for parses that cannot reach the top n. It is the "Rising Sun"
  model. A threshold was rejected because the UI expects a fixed number of
  results. ([post](https://mitcho.com/blog/observation/scoring-for-optimization/))

### Where the docs and the code disagree

| Topic | Docs | Code (`11dc94e`) |
| --- | --- | --- |
| arbitrary text score | example uses 0.7 | 0.3 (`noun_arb_text`) |
| step 10 formula | written as a product of noun probabilities | additive `m + Σ score·m` |
| maxScore pruning | "has yet to be implemented" (mitcho) | implemented (`addIfGoodEnough`) |
| suggestion memory | 0.6 plans "cover noun suggestions" | verbs only |
| `rankLast` | Parser 1's specific-vs-generic flag | not read by Parser 2 |
| `en` roles | tutorial still shows `position` | `location` + `time` |

The code is authoritative for a port. The docs explain why.

### Known limits, from the authors

- **Strongly case-marked languages are out of scope on purpose.** The tutorial:
  *"encourage the use of adpositions."* German works, because case sits on
  determiners.
- **Clitics** were designed (treat them like the selection) but never built.
- **Unmarked arguments** (bare times) get no role.
- **One argument per role.** `share-on-delicious` wanted two `alias` arguments,
  and that was left open.
- **Candidate lists grow geometrically** with ambiguity.
- **Argument-first** makes it hard to know what arguments to type.

### Jono's retrospective (Jan 2010)

[Retrospective](https://jonoscript.wordpress.com/2010/01/20/retrospective-what-we-learned-from-ubiquity/):

- *"This crazy thing can work"* (about 500k users) and *"can be localized"*.
  But localization must be *"practical and useful… not just an academic
  linguistics exercise."*
- The hotkey overlay *"suffers from lack of visibility"*: *"I keep forgetting
  that it's there."*
- *"Nountypes are very powerful… but hard to get people to use well"*: they
  were invisible, so authors re-invented them per command.
- **Namespace collisions** were never solved.
- **"Inputs are not just strings."**
- *"Looking at a preview… can be more useful than actually doing it."*
- *"It's not the system that is valuable to users, it's the individual
  commands."*

### Taskfox, the successor that dropped the parser

[Taskfox](https://wiki.mozilla.org/Taskfox) *"will not feature natural language
processing."* It moved modifiers out of the typed sentence and into the
preview UI, because that is more discoverable and localizable. It did so
*"at the cost of effortlessly typing what you want to do."*

### What this adds for the port

- **Noun invisibility was Ubiquity's own diagnosis of failure.** Attaching
  `noun!` to concepts, which already carry descriptions, views and `label`
  facets, answers it directly. A concept is visible, and every command over it
  reuses the one declaration.
- **Taskfox is a warning, not a model.** It dropped typed arguments for
  discoverability and lost what made Ubiquity worth using. The source shows
  the trade was never necessary. Argument finding (`argFinder`) is about 300
  lines driven by a 20-line language file. The costly parts are noun
  detection, async and scoring, and a palette needs those however arguments
  are entered. Ubiquity already handled the discoverability case inside the
  sentence: an unfilled role renders its label (`rename [notebook] to
  [text]`), and defaults fill it at half score. The port keeps typed
  arguments from the start.
- **"Inputs are not just strings"** is where tonk is ahead. A selected block or
  row is an entity, so it can be a direct noun hit instead of text re-parsed by
  noun types.
- **Visibility.** The palette lives in the FABB, which is always on screen,
  rather than behind a hotkey alone. That answers the "I keep forgetting it's
  there" failure.

## What the grammar did not have

Ubiquity's grammar is **one verb plus role-marked arguments**. That is the whole
of it. There are no adjectives, adverbs, quantifiers, negation, conjunction or
chaining ("… and then …"). mitcho's paper states the scope deliberately:
*"simply… composed of a single verb and its arguments"*.

The pieces that sit nearest to those parts of speech:

| Looks like | What it really is |
| --- | --- |
| adverbials ("tomorrow", "with google", "in German") | the `time`, `instrument` and `format` **roles**: prepositional arguments of the verb. `format` is used by `Wikipedia` (language) and the Amazon search. |
| "of" / "for" phrases | the `modifier` role, the one role that is an argument of a *noun* rather than the verb. The Semantic Roles page calls it *"fundamentally different"*. No stock command in the standard feeds uses it. |
| pronouns | `anaphora` ("this", "it", …), always resolved to the selection. |
| clitic pronouns (*Envoyez-le*) | `clitics` declared in `ca`, `es`, `fr` and `it` (`{clitic: 'le', role: 'object'}`), but step 3 of the pipeline is unimplemented, so they are never read. |
| articles ("the", "el") | stripped by `normalizeArgument`; they carry no meaning. |
| a fixed argument ("to English") | `CreateAlias` with `givenArgs` (`anglicize`), or a separate verb. |
| qualities ("high contrast", "bold") | either verbs (`bold`, `italicize`) or closed-set noun types filling a role. |

Parser 1 had `takes` (the direct object) and `modifiers` keyed by English
preposition. Parser 2 replaced these with roles, and Parser 1 commands were
refused ("not compatible with Parser 2").
