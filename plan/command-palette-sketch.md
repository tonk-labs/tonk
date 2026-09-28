# Command palette: Ubiquity, ported to dialog

Status: sketch. `grammar!`, `noun!` and `verb!` are **hypothetical syntax**.
`command!`, `concept!`, `view!` and `rule!` are the library's real shapes.
Based on the source study in [command-palette-ubiquity.md](command-palette-ubiquity.md).
Supersedes the earlier `phrase!`/`suggest!` draft of this file, which replaced
Ubiquity's role grammar with per-command templates and so lost the property
that made Ubiquity localize for free.

## The port in one table

Nothing new becomes the vocabulary. Tonk's **commands** are what runs, and its
**concepts** are what arguments are. The three declarations are annotations on
them, plus one per-language grammar, exactly as Ubiquity had:

| Ubiquity | Tonk | Attached to |
| --- | --- | --- |
| language parser (`en.js`: roles, delimiters, branching, anaphora) | `grammar!` | a locale |
| noun type (`suggest`, `default`, `label`, cache flush) | `noun!` | a concept, or a built-in value type |
| `CreateCommand` (`names`, `arguments`, `preview`, `execute`) | `verb!` | an existing `command!` |
| `preview(pblock, args)` | a `preview` view facet | the command concept |
| `execute(args)` | asserting the command transient | (nothing new) |
| `CreateAlias({givenArgs})` | a `verb!` with `given:` | the same command |
| `registerCacheObserver(flush)` | the dialog subscription behind the noun's query | (free) |
| suggestion memory table | `palette/choice` facts on the profile branch | the profile |
| the selection | the DOM selection, and `{this}` from `tonk:site` | the page |

`verb!` and `noun!` never define a concept, and never touch a command's or
concept's identity. They are keyed by name like `event!`, which is what lets
tonk ship them in the built-in layer for spaces seeded long ago.

## `grammar!`: one per locale

The whole of `en.js`, as data:

```yaml
grammar!: &grammar/en
  locale: en
  branching: right               # "to bob": argument follows its delimiter
  spaces: true
  verb:
    initial: 1.0                 # verbInitialMultiplier
    final: 0.3                   # verbFinalMultiplier
  anaphora: [this, that, it, selection, him, her, them]
  roles:
    goal:       [to]
    source:     [from]
    location:   [near, on, at, in]
    time:       [at, on]
    instrument: [with, using]
    format:     [in]
    modifier:   [of, for]
    alias:      [as, named]
```

And Japanese, which the same commands then parse in without being touched:

```yaml
grammar!: &grammar/ja
  locale: ja
  branching: left                # "ボブに": argument precedes its particle
  spaces: false                  # word breaking inserts U+200B around particles
  verb:
    initial: 0.3
    final: 1.0                   # verbs are sentence-final
  anaphora: [これ, それ, あれ]
  roles:
    object:     [を, と]
    goal:       [に, へ]
    source:     [から]
    time:       [に]
    location:   [で, に]
    instrument: [で]
    alias:      [として]
    modifier:   [の]
    format:     [で]
```

The parser is generic Rust code. It reads the active locale's grammar from
the registry, and a `grammar!` asserted on a profile branch overrides the
built-in one for that person.

## `noun!`: a concept as a noun type

Ubiquity's `suggest(text) → [{text, html, data, summary, score}]` becomes a
declaration the parser executes against dialog:

```yaml
noun!: &noun/member
  concept: tonk:member
  label: member                  # shown for an unfilled argument: "expel [member]"
  match: [name]                  # stored text fields that typed text is scored against
  # text: the `label` view facet as plain text; html: the `label` facet
  # data: the entity (what reaches the verb)

noun!: &noun/space
  concept: space
  label: space
  match: [name]
  where: { kind: tonk:repository }   # never offer the profile's self-replica

noun!: &noun/notebook
  concept: notebook/named
  label: notebook
  match: [title]
```

For a typed argument `text`, the parser:

1. Queries the concept with `where:` plus a `StartsWith` pushdown on the first
   `match` field (an index range), and a bounded scan for substring hits.
2. Scores each hit with Ubiquity's `matchScore`:
   `0.3 + 0.25·√(matched / typed) + 0.45·(1 − index / typed)`.
3. Returns suggestions whose `data` is the entity and whose `text`/`html` come
   from the concept's `label` view.

Its results are cached per (text, noun). The cache is dropped when the
subscription on the concept reports a change. That is Ubiquity's
`registerCacheObserver`, which `noun_type_tab` had to hand-wire to
`TabOpen`/`TabClose`, and it comes free here.

**Closed sets** (`NounType({Afrikaans: "af", …})`) are concept instances, for
example a `tonk:role` concept with `founder` and `member` rows. They are facts,
so a space can add to them.

**Recognizers** (`noun_arb_text`, email, URL, number, date) are built-in value
nouns implemented in Rust, with Ubiquity's scores: arbitrary text is 0.3,
anything else 1.

**Unions** (`mixNouns`) are rules concluding a concept:

```yaml
concept!: &nameable
  description: Anything with a name the palette can rename.
  with:
    name: { the: xyz.tonk.palette.nameable/name, as: text }

rule!:
  assert: nameable
  when:
    - assert: notebook/named
      where: { this: ?this, title: ?name }

noun!: &noun/nameable
  concept: nameable
  label: thing
  match: [name]
```

**Defaults** (Ubiquity `default`, used for an unfilled role at half score)
are a query or a context binding:

```yaml
noun!: &noun/space-here
  concept: space
  label: space
  match: [name]
  default: "{space}"             # the active space
```

## `verb!`: a command made sayable

This is Ubiquity's `CreateCommand` minus `execute`. Executing is asserting the
command the verb annotates.

```yaml
verb!: &verb/expel
  command: member/expel
  names: [expel, remove member, kick]
  description: Remove a member from this space.
  help: Try "expel alice". They keep their copy and stop receiving changes.
  arguments:
    object:
      noun: noun/member
      field: member              # the command field this role fills
      value: member              # project the entity to its `member` field (the DID)
  when:                          # the verb is offered only if this matches
    - assert: tonk/member-role
      where: { member: "{profile}", role: tonk:founder }

view!:
  this: member/expel
  show:
    preview: |
      remove <tonk-display entity={object.data} model=tonk:member view=label></tonk-display>
      from this space
```

- **`arguments` is keyed by role**, and there is at most one argument per role,
  as in Ubiquity. The connecting words come from `grammar!`: in English, "expel
  alice" is `object`.
- **`field`/`value` is the one addition Ubiquity did not need.** Ubiquity's
  `execute` received `{text, html, data}` and wrote its own code. A tonk
  command has handler-shaped fields, so the verb says how an argument's `data`
  lands in them. Fields no role fills come from `fill:` with context
  (`{now}`, `{space}`), the same mini-language `event!` uses.
- **`preview` is a view facet on the command concept.** It is rendered with the
  highlighted parse's arguments available as `{role.text}`, `{role.html}` and
  `{role.data}`, 150 ms after typing stops (Ubiquity's `previewDelay`). With no
  facet, the preview is the description, which is Ubiquity's
  `previewDefault`.
- **`when:`** has no Ubiquity counterpart. Ubiquity offered every verb
  everywhere and let `execute` fail. Here applicability is a query premise.

### Two commands, one word

```yaml
verb!: &verb/rename-notebook
  command: notebook/retitle
  names: [rename, retitle]
  arguments:
    object: { noun: noun/notebook, field: subject }
    goal:   { noun: text,          field: title }

verb!: &verb/rename-space
  command: tonk/rename-repository
  names: [rename]
  arguments:
    object: { noun: noun/space-here, field: space, value: subject }
    goal:   { noun: text,            field: name }
```

Ubiquity did not special-case this, and neither does the port. Both verbs match
"rename", and `argFinder` produces the same parses for both. Noun detection
then decides: a role whose noun returns nothing for its text kills that parse.
"rename roadmap to Q3" survives only as a notebook rename if "roadmap" is a
notebook title.

### An alias

Ubiquity's `anglicize` is `translate` with `givenArgs: {goal: "English"}`. The
same shape in tonk is a verb whose role is fixed and hidden:

```yaml
verb!: &verb/leave
  command: space/remove
  names: [leave, leave space]
  description: Leave the space you are in.
  arguments:
    object: { noun: noun/space, field: subject, value: subject }
  given:
    object: "{space}"            # run through the noun as if typed; hidden in display
```

## The walk, with Ubiquity's numbers

Context: English grammar, the palette opened on notebook `N` ("Roadmap"), no
text selected. The registry holds the verbs above. `m` is the parse's score
multiplier, and `score = m + Σ roleScore·m`.

**`rename this to Q3 plan`**

1. **Verb finding.** "rename" is an exact match for both rename verbs:
   `0.4 + 0.6·√(6/6) = 1.0`, verb-initial ×1.0. Memory can only raise a score
   that is already 1.
2. **Argument finding.** The argument string is "this to Q3 plan". The only
   delimiter is "to" (goal), so the power set gives:
   - `{object: "this to Q3 plan"}`;
   - `{object: "this", goal: "Q3"}` with "plan" as a second object, which
     `suggestArgs` kills;
   - `{object: "this", goal: "Q3 plan"}`.
3. **Anaphora.** "this" is an anaphor. The port substitutes `{this}` = `N`,
   which is what Ubiquity did with the selection, and applies ×1.2, so
   `m = 1.2`.
4. **Noun detection.**
   - `noun/notebook` on `N` is a direct entity hit, score 1.
   - `noun/space-here` on `N` returns nothing, so the space parse dies.
   - `text` on "Q3 plan" scores 0.3.
   - `noun/notebook` on "this to Q3 plan" returns nothing, so that parse dies.
5. **Score.** `notebook/retitle {object: N, goal: "Q3 plan"}` scores
   `1.2 + 1·1.2 + 0.3·1.2 = 2.76`.

**`rename Q3 plan`**

1. Two parses survive, both with `m = 1.0`:
   - notebook rename: `object` "Q3 plan" must be a notebook title, and none
     matches, so it dies unless one does;
   - space rename: `object` "Q3 plan" as a space name dies unless a space is
     called that.
2. With nothing matching, **applyObjectsToOtherRoles** does *not* move "Q3
   plan" to `goal`. The verb is known and takes an object, which is the rule
   that kept "twitter hello" from becoming "twitter as hello".
3. The palette shows nothing better than "rename [thing] to [text]". That is
   faithful, and it is the case where the `{this}` default (below) matters.

**`ren`**

1. A prefix match on both rename verbs: `0.4 + 0.6·√(3/6) ≈ 0.82`.
2. There is no argument string, so both parses fill their roles with defaults
   at half score:
   - the space verb's `object` defaults to `{space}` (0.5);
   - the notebook verb has no default for `object`, so it gets the empty
     suggestion (0.5, with no text) and shows as "rename [notebook] to [text]".
3. The space rename ranks first: "rename *Budget* to [text]". **This is a real
   finding.** Ubiquity would suggest renaming the space while you're looking at
   a notebook, because only the space noun has a default. The port should give
   `noun/notebook` a `default: "{this}"` whenever `{this}` conforms, which
   Ubiquity approximated with selection interpolation.

**`alice`** (noun-first)

1. No verb prefix matches "alice".
2. `argFinder` with no verb makes `{object: "alice"}`, and `suggestVerb` copies it
   for every verb that takes an `object` with a local noun, at `m = 0.3` and a
   verb score of `1 − 0.7/(1 + timesUsed)`.
3. `noun/member` matches "Alice" at 1.0, and `noun/notebook` matches nothing.
   Only "expel Alice" survives (for an admin), plus "open Alice" if an open
   verb takes members.

**`これを「Q3計画」に名前変更`** (Japanese)

1. The only change is `.po`-style names for the same two verbs:
   `names@ja: [名前変更, 名前を変更, 名前を変更する, …]`.
2. `ja.wordBreaker` splits the input at を and に.
3. The verb is found sentence-finally (×1.0 in Japanese). `これ` is an anaphor,
   so it becomes `N`, and `を` marks it as `object`. 「Q3計画」 with `に` is the
   `goal`.
4. It is the same `notebook/retitle` claim. No verb or command declaration
   changed.

## Selection

Ubiquity's strongest argument source was the selection. It was interpolated
into every parse, substituted for anaphora, used for noun-first, and it drove
the no-input context menu. Tonk has two candidates, and they should not be
conflated:

- **A real DOM selection** (selected text in a notebook block, selected table
  cells) maps 1:1 to Ubiquity's selection. It is interpolated as an `object`
  argument ×1.2, substituted for anaphora, and offered as noun-first input.
  The selected *text* goes through the noun types like any typed text. A
  selected *entity* (a block, a row) is a direct hit.
- **`{this}`, the entity the page shows**, is ambient rather than chosen. If it
  were treated as a selection, every parse would get it interpolated as an
  object. It should resolve anaphora ("this", "it") and act as a noun
  `default` (half score), so an explicit argument always outranks it.

## Memory

Ubiquity remembered (typed verb prefix → chosen verb) and ("" → verb), and
nothing about nouns. The port stores the same rows as facts on the profile
branch, so they sync across devices:

```yaml
concept!: &palette/choice
  with:
    input: { the: xyz.tonk.palette.choice/input, as: text }    # typed verb prefix, "" for global
    verb:  { the: xyz.tonk.palette.choice/verb,  as: entity }
    count: { the: xyz.tonk.palette.choice/count, as: unsigned-integer }
```

The verb score gets the same `score^(1/(1 + count))` boost. Noun memory (typed
text → chosen entity) is the obvious extension. It would be a second row
shape, and the parser would multiply a noun's suggestion score the same way.

## The Rust side

```rust
pub struct Registry {
    grammar: Grammar,                // active locale's `grammar!`
    verbs: Vec<Verb>,                // `verb!` rows, all layers
    nouns: HashMap<Entity, Noun>,    // `noun!` rows + built-ins
}

pub struct Grammar {
    branching: Branching,
    spaces: bool,
    verb_initial: f64,
    verb_final: f64,
    anaphora: Vec<String>,
    roles: Vec<(Role, String)>,      // (role, delimiter), many-to-many
}

pub struct Verb {
    command: CommandDescriptor,      // asserted inline, as the FAB's claim builders do
    names: Vec<String>,
    arguments: BTreeMap<Role, Argument>,
    given: BTreeMap<Role, String>,
    applies: Option<Premises>,
}

pub struct Argument { noun: NounRef, field: String, value: Option<String>, label: Option<String> }

#[async_trait(?Send)]
pub trait NounType {
    /// Ubiquity's `suggest`: zero or more scored interpretations of `text`.
    async fn suggest(&self, text: &str, cx: &Context) -> Vec<Suggestion>;
    async fn default(&self, cx: &Context) -> Vec<Suggestion>;
}

pub struct Suggestion { text: String, html: String, data: Value, score: f64 }
```

The pipeline steps (word break, verb finding, argument finding with power
sets, anaphora, normalization, objects to other roles, verb suggestion, noun
detection with a (text, noun) cache, the argument cartesian product, scoring,
and `maxScore` pruning) are pure functions of `Registry`, `Context`, input and
`NounType` results. They can be tested natively against fake noun types,
including Ubiquity's own `test_parser2.js` cases ported as fixtures.

## Where the port departs from Ubiquity, and why

| Ubiquity | Port | Reason |
| --- | --- | --- |
| `execute(args)` code | assert the annotated command | commands and handlers already exist; the worker runs only transients the page asserts |
| verbs offered everywhere | `when:` premise | applicability is a query we can afford |
| selection only | DOM selection plus `{this}` as anaphor/default | the page has an entity even with nothing selected |
| noun cache flushed by hand-wired observers | flushed by dialog subscription | free |
| memory in local SQLite | facts on the profile branch | syncs |
| `rankLast`, `noSelection` flags | dropped | dead in Parser 2 already |
| `DONT_PARSE_MULTIPLE_ARGS_PER_ROLE` | kept on | Ubiquity turned it on for speed; nothing needs several arguments per role |

## Open

- **Rendering a preview over uncommitted overlay facts** is a new mode for
  `<tonk-display>`.
- **The FABB frame can't see the page's `{this}` or its selection today.** Both
  have to travel with the key relay.
- **A concept noun's substring scan** needs a bound. `StartsWith` covers prefix
  matches only.
- **Locale names for verbs** (`names@ja`): as per-verb facts keyed by locale,
  or as a separate `.po`-like translation concept.
