# One index, one vocabulary, one evaluator

**Goal:** Give the `tonk` CLI a mental model a caller can reason from. After
reading the one-screen index once, a person or an agent should be able to
guess what any command does, what its arguments look like, and where to look
when a guess fails — the way `git` and `gh` read on first contact.

**Approach:** Collapse the surface behind the primitives it already has.
Nothing here adds a capability. Every item is a place where the CLI presents
a spelling helper as a peer of the thing it helps with, or answers one
question several ways, and the fix is to pick the primitive and make the
rest read as sugar over it.

**Scope:** Implementation only. The old spellings stop resolving and are not
aliased; the callers elsewhere in the monorepo that name them (item 9) are
updated after this lands, not as part of it.

**Status:** implemented on `feat/cli-cleanup` (2026-08-25).

---

## 0. The surface today

Twenty-five visible top-level commands, and bare `tonk` prints a 72-concept
dump (`tonk context`) rather than an index. Two things make it feel
structureless.

**There is one primitive and it is presented as one of many.** `concept
add`, `view add`, `home`, `assert`, `retract` and `query` each build a string
of notation (`authoring.rs`, `data.rs`) and hand it to
`eval::run_against_site`. Evaluating a notation document against a branch is
the whole of what the data side does; the verbs are macros over notation
shapes. Presented as peers of `eval`, they read as six operations with three
argument grammars (`concept add <name> --attr f:t:c`, `view add <concept>
--template`, `home <concept>…`, `assert <concept> [<entity>] --<field>`)
instead of one operation spelled six convenient ways.

**"Where am I / what is here" has nine spellings.** `context`, `status`,
`schema`, `guide`, `agents`, `space use` (bare), `account status`, `concept
ls`, `view ls` — with four listing grammars (`ls`, `list`, `spaces`, bare)
across the nouns.

## 1. Five rules

Each is borrowed from git, and each makes a command guessable from any other.

1. **One language, one evaluator.** `eval` is the plumbing. Every other data
   command is a macro that expands to notation, and says so: every write verb
   takes `--notation`, which prints the document it would evaluate and stops.
   The sugar teaches the language instead of hiding it.
2. **Noun commands share one shape.** Bare lists; `add` and `rm` mutate;
   there is no `ls` or `list` subcommand anywhere. `git branch`, `git remote`,
   `git tag`.
3. **Verbs take `<concept> [<entity>]`, then `--<field>` flags.** `assert`,
   `retract`, `query` already do; `show` and `render` follow.
4. **`--json` everywhere**, including `eval`. No `--format`.
5. **Help is the manual.** Bare `tonk` and `tonk -h` print the same grouped
   index. `tonk help <command>` is one command; `tonk help <guide>` is one
   guide; `tonk help -a` is every command; `tonk help -g` is every guide.

## 2. Bare `tonk` and `tonk -h` print the index

```text
usage: tonk [--space <name>] [-v] <command> [<args>]

A space is a synced store of facts about entities. A concept is a schema:
an entity that matches one is an instance with typed fields. Views render
instances. Reads and writes are notation, evaluated against the space
(see 'tonk help notation').

start a space (see also: tonk help spaces)
   space      List spaces, create one, or bind this directory to one
   join       Join a shared space from an invite URL

examine state
   status     Where you are: space, branch, sync, account
   show       Describe the schema, a concept, an entity, or a view
   query      Read the instances of a concept
   render     Render a view to HTML

write facts
   assert     Create an instance of a concept, or update fields on one
   retract    Retract a field, or a whole instance
   eval       Evaluate a notation document: anything the verbs can't say

define
   concept    List concepts, or define one with typed fields
   view       List views, or author one for a concept

collaborate (see also: tonk help sync)
   invite     Mint an invite URL granting access to this space
   pull       Pull main from its upstream
   push       Push main to its upstream
   remote     List or manage remotes
   account    Sign in to a Tonk account; manage devices and spaces

'tonk help -a' lists every command; 'tonk help -g' lists the guides
(glossary, notation, views, events, workspace). See 'tonk help <command>'
or 'tonk help <guide>' for details.
```

Fifteen commands. Not shown, but listed by `tonk help -a` and resolvable:
`blob`, `export`, `import`, `telemetry`, `update`, `migrate`, `identity`,
and the `space home` / `space agents` subcommands (item 6). Git hides
`bundle` and `fast-export` the same way.

**The preamble does the vocabulary work.** It introduces fact → entity →
concept → instance → field → view once, in that order, so every line below
can say "instance" and "field" without defining them. `show` deliberately
says "entity": it is the one command that reaches below concepts (item 5).
The index never says "mint", "supersede", "attribute", "claim", "rule" or
"join"; those are `tonk help assert` and glossary words (item 8).

**Implementation.** clap 4 has no notion of command groups, so the index is
a static string installed with `Command::override_help` (and `help_template`
for the usage line), not generated from the derive. That makes drift the
risk, so one test guards it both ways: every non-hidden `Command` variant
appears in the index by name, and every command name in the index parses.
Bare `tonk` (no subcommand) prints the same string and exits 0.

`help` becomes tonk's own subcommand (`disable_help_subcommand`, a `Help`
variant): `tonk help <command>` renders that command's clap help, `tonk help
<guide>` prints the guide, `-a` renders the full clap command list including
hidden ones, `-g` lists guides with one line each. A name that is neither
reports both lists.

## 3. `help` absorbs `guide`

`tonk guide [<topic>]` is `git help <concept>` under another name. It goes.

| today | becomes |
|---|---|
| `tonk guide` | `tonk help -g` (the guide list) |
| `tonk guide notation` etc. | `tonk help notation` |
| `tonk guide views tonk-table` | `tonk help tonk-table` — each built-in element is its own guide |
| `tonk guide all` | `tonk help all` — kept because harnesses prime agents with it |

`guide-index.md` stops existing as a document and splits into three guides:

- **`glossary`** — the "Mental model: it's a datalog" section, plus the two
  gotchas it already carries (content-addressed identity, quoted strings).
  This is where instance, field, attribute, claim, mint and supersede are
  defined, and it is listed first in the footer because it is what to read
  before `notation`, which is the grammar reference.
- **`spaces`** — the "Spaces" section: resolution order, `--space`,
  `TONK_SPACE`, `space use`/`unbind`. The index's "see also" for the start
  group.
- **`tutorial`** — "The loop": discover, define, write, view, look, share.
  The empty-space workflow that `tonk context` prints today lives here.

Plus one new guide, **`sync`**: upstream, remotes, auto-sync around writes,
`--no-sync`, invite/join. The index's "see also" for the collaborate group.
Today that story is spread over `push`, `pull`, `remote`, `invite` help
text and item 5 of `plan/cli-consistency.md`.

`guide::TOPICS` becomes the single list `help -g` prints, the footer cites,
and the drift test checks.

## 4. `status` is the one "where am I"

`plan/cli-consistency.md` item 1 folded `status`, `space use` and `account
status` into `context` as sections, and left `context` offline so that bare
`tonk` would not fetch. With bare `tonk` now printing help, `context` has no
reason to exist: its orientation sections are `status`, and its concept
listing is `concept` (item 6) and `show <concept>` (item 5).

```text
$ tonk status
space:    tonk-team-recovered  (directory /Users/jack/tonk/tonk)
branch:   main  #6Mg1NtTb…
upstream: synced with prod
account:  did:key:z6Mkqzcp…  (accounts-staging.tonk.xyz, synced)
device:   did:key:z6Mkmr5s…
```

`status` fetches — that was the resolved answer to the one open question in
the consistency plan, and the reason it was a separate command. `--json`
emits `tonk.status.v2`: the `space`, `sync` and `account` objects that
`tonk.context.v3` carries today, without `concepts` and
`emptySpaceWorkflow`. `sync.fetched` stays so a reader can tell an
unreachable upstream from a synced one.

`account status` stays — it is the account noun's bare form (item 6) and
its `--json` is a projection of the same account object. `space use` with
no argument stops reporting; bare `space` lists with the active one marked.

The per-concept "inspect / update / create" recipes that `context` prints
are the useful part of that screen; they move to `show <concept>` where an
agent looking at one concept can find them.

## 5. `show` describes one thing

`git show` takes any object. `tonk show` takes any name and dispatches on
what the name resolves to.

| form | prints | replaces |
|---|---|---|
| `tonk show` | the branch's schema as re-submittable notation | `tonk schema` |
| `tonk show <concept>` | fields with type and cardinality, the description, views over it, and the recipes (`query`, `assert` create and update forms) | `tonk schema <concept>`, `tonk assert <concept> --help`'s table, `context`'s per-concept block |
| `tonk show <view>` | model, anchor, and the template | a `view ls` row |
| `tonk show <entity>` | every fact on the entity, grouped by concept where one matches, raw attribute otherwise | `tonk query <concept> <entity>` |

Resolution goes through the name table: a concept name, a view anchor and an
entity bookmark are all names, and a `did:key:` URI is an entity. A name that
resolves to more than one kind prints each section. `--json` on every form;
`--notation` on the schema and concept forms emits the re-submittable subset
that `tonk schema` prints today (the "no `--json`, deliberately" reasoning
from the consistency plan holds for that output, so it moves behind a flag
rather than being transcribed).

**`show <entity>` needs a spike before it is promised.** The other three
forms are re-spellings of things that exist. Showing an entity without naming
its concept means querying with an open attribute position, and whether the
evaluator plans that over the analyzer's concept-shaped queries is not
established. If it does not, the form is `tonk show <concept> <entity>` and
the summary line says so; it is still one `show`, and `query <concept>
<entity>` still goes.

`render` is unchanged apart from its summary.

## 6. Noun commands: bare lists, no `ls`

| noun | bare | subcommands |
|---|---|---|
| `space` | list, active marked | `new`, `use <name>`, `unbind`, `rm`, `link`, `home <concept>…`, `agents [get\|set]` |
| `remote` | list | `add`, `set-upstream` |
| `concept` | list | `add <name> --field f:t:c…` |
| `view` | list | `add <concept> --template …` |
| `blob` | list | `add`, `cat` |
| `account` | status | `login`, `logout`, `status`, `sync`, `delete`, `devices`, `revoke`, `space [pull\|delete]` |

Removed: `space list`, `remote list`, `concept ls`, `view ls`, `blob ls`,
`account space list`, and the plural `account spaces`. No aliases — an alias
keeps the second grammar alive, which is the thing being fixed. Listing output
keeps the one renderer from consistency item 0.4.

**`home` moves under `space`.** It pins concept directories on the space
home and re-points the `tonk/space` alias: a property of the space, not a
peer of `assert`. `view add` still auto-surfaces the first view when no home
is set, which is why it is fine for `space home` to sit off the index.

**`agents` moves under `space`.** One claim on the repository subject; `tonk
space agents` prints it, `tonk space agents set <file>` writes it.

**`concept add --attr` becomes `--field`.** Under the preamble's vocabulary,
a concept's slots are fields and the predicates under them are attributes.
`assert` already says `--<field>`; the definition and the use should agree.
The `f:t:c` value grammar is unchanged.

## 7. Write verbs: plain words, and `--notation`

**Summaries use the reader's words.** `assert` is "Create an instance of a
concept, or update fields on one" — not "Mint … or supersede". `git commit`
is "Record changes to the repository", not "create a commit object". What
makes mint and supersede the precise words — content-addressed identity, a
cardinality-one assert replacing the prior value — is explained in `tonk
help assert` and the glossary, where there is room.

**`--notation` on every macro verb**: `assert`, `retract`, `concept add`,
`view add`, `space home`. Prints the document the verb would evaluate and
exits 0 without evaluating. Distinct from `--dry-run`, which runs the
analyzer and the queries and drops the transaction. Cheap: the builders in
`authoring.rs` and `data.rs` already return the string. `assert`'s copy is
built by `data_ops::flags` like its other switches, since everything after
`<CONCEPT>` reaches it raw.

**`eval --format json` becomes `eval --json`** (rule 4). `-c`, the
positional path and `-` are unchanged.

## 8. Vocabulary

Where each word may appear. The index and command summaries use the left
column only; the right column is for `tonk help <command>` bodies, error
text, and the guides.

| index and summaries | help bodies and guides |
|---|---|
| space, branch, upstream, remote | site, registry, binding |
| fact, entity | attribute, claim, assertion, retraction |
| concept, instance, field | schema (as a synonym once, in the preamble), anchor, `this:` |
| view, render | template, model, route, directory |
| account, device | root, delegation, UCAN, session |
| create, update, retract | mint, supersede |
| notation | rule, join, effect, transient |

"Attribute" survives in the notation itself (`attribute!:`) and in the
glossary, which is where a reader learns that a field is an attribute seen
through a concept.

## 9. What stops resolving

Not migrated here. Listed so the follow-up elsewhere in the monorepo has an
inventory rather than a search.

**Spellings removed:** `tonk context`, `tonk guide`, `tonk schema`, `tonk
home`, `tonk agents`, `tonk concept ls`, `tonk view ls`, `tonk blob ls`,
`tonk remote list`, `tonk space list`, `tonk account space list`, `tonk
account spaces`, `tonk query <concept> <entity>`, `tonk eval --format`, `tonk
concept add --attr`, bare `tonk space use` as a report.

**Contracts retired:** `tonk.context.v3`. Its sections continue as
`tonk.status.v2`, `tonk concept --json`, and `tonk show <concept> --json`.

**Callers found outside the crate** (`grep` for the spellings above, 2026-08-25):

```text
.claude/commands/tonk.md
.claude/skills/tonk-bug/SKILL.md
README.md
rust/tonk-cli/README.md
guide/src/introduction.md
guide/src/reference.md
bench/README.md
bench/EXPERIMENTS.md
bench/bin/handoff.sh
bench/bin/shots.sh
bench/scenarios/{agents-handoff,artifact-conversion,first-use,from-scratch,
                 interview-build,smoke,targeted-edit,wiki-conversion}/*
```

Nothing in `tonk-ui`, the account service, or `harness/` matched. Inside
the crate, hint strings in `account.rs`, `account_spaces.rs`,
`account_state.rs`, `inventory.rs`, `invite.rs`, `recovery.rs`, `remote.rs`,
`site.rs`, `space.rs`, `sync.rs` and the guide `.md` files name the old
spellings and are swept as part of each item, not afterwards — a hint that
names a command which no longer exists is the failure consistency item 0.1
was about.

**Tests touching the old surface:** `tests/context.rs`, `schema_read.rs`,
`authoring.rs`, `data_verbs.rs`, `agents.rs`, `space.rs`, `cli_space.rs`,
`space_inventory.rs`, `notation.rs`, `telemetry.rs`.

## 10. Order

Slices on `feat/cli-cleanup`, each leaving the crate green and each usable
on its own. The first is the only one that is not breaking.

1. **Item 2 and 3** — the index as override help, bare `tonk` prints it,
   `help` subcommand with `-a`/`-g`, guides split into `glossary`,
   `spaces`, `tutorial`, `sync`, elements as topics, drift test. `guide`
   is removed in this slice; it has nothing left to print.
2. **Items 7 and 8** — summary and hint vocabulary sweep, `--field`,
   `--notation`, `eval --json`.
3. **Item 6** — bare nouns list, `ls`/`list` removed, `home` and `agents`
   under `space`.
4. **Item 4** — `status` absorbs `context`'s sections; `context` removed;
   `tonk.status.v2`.
5. **Item 5** — `show`, in two steps: the schema, concept and view forms
   (which retire `schema`), then the entity form after its spike decides
   its shape (which retires `query <concept> <entity>`).

Then the monorepo follow-up from item 9.
