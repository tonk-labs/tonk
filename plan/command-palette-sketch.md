# Command palette: the design, and the proof of concept

Status: proof of concept on `worker/adoring-franklin-kd80e0` in tonk and
dialog-db. Supersedes the earlier drafts of this file. The Ubiquity study it
rests on is [command-palette-ubiquity.md](command-palette-ubiquity.md).

## Shape

Commands and concepts stay the vocabulary. The palette adds four relations
about them, all in existing notation (concepts, instance assertions, a rule,
views). There is no new syntax and no worker API.

| Ubiquity | Here |
| --- | --- |
| `CreateCommand({names})` | `palette/verb` — words asserted on the command itself |
| `arguments: [{role, nountype}]` | `palette/argument` — a command field (its attribute), a role (an entity), a noun (a concept) |
| noun type `label` | `palette/noun` — words asserted on the concept itself |
| noun type `suggest(text)` | the concept's rows, matched by their `label` view facet |
| `en.js` roles and delimiters | the grammar in `dialog-palette` (Rust), roles as `palette/role` entities |
| `execute(args)` | transacting the command, with its fields' own selectors |
| the selection | the entity the page shows (`{this}`): anaphor target and default |

## The schema (core library)

```yaml
concept!: &palette/verb
  description: Words that say a command. Asserted on the command itself.
  with:
    name: { the: xyz.tonk.palette.verb/name, cardinality: many, as: text, … }

concept!: &palette/noun
  description: Words for a kind of thing. Asserted on the concept itself.
  with:
    name: { the: xyz.tonk.palette.noun/name, cardinality: many, as: text, … }

concept!: &palette/role
  with:
    name: { the: xyz.tonk.palette.role/name, cardinality: one, as: text, … }

concept!: &palette/argument
  with:
    command: { the: xyz.tonk.palette.argument/command, as: entity, … }
    field:   { the: xyz.tonk.palette.argument/field,   as: entity, … }   # an attribute
    role:    { the: xyz.tonk.palette.argument/role,    as: entity, … }   # a palette/role
  maybe:
    noun:    { the: xyz.tonk.palette.argument/noun,    as: entity, … }   # a concept

palette/role!: &palette/object
  name: "object"
# goal, source, location, time, instrument, format, modifier, alias
```

(The library spells each field out in full; the braces here are
abbreviation.)

- **Verbs and nouns are assertions on the thing they name**, one per word, so
  words can be added or retracted independently.
- **An argument is a relation**, derived from its body: attributes are
  shared between commands (`space/create` and `space/enable-sync` both use
  `create-space/name`), so the pair (command, field) is what an argument is
  about.
- **A field is its attribute entity.** A command declares the field as a
  named `attribute!` and references it from `with:`; the descriptor, and so
  the command's identity, is unchanged.
- **A noun is a concept.** Its rows are the candidates; the chosen row's
  entity is the field value. Where a command wants something other than a
  concept's row, a rule derives a concept whose rows are exactly that —
  `member/account` below.

## One command, end to end

```yaml
attribute!: &expel-member/member
  description: The DID of the member to remove.
  the: xyz.tonk.command.expel-member/member
  as: entity

command!: &member/expel
  description: Remove a member from this space.
  with:
    member: expel-member/member

# The roster row is the membership; the command takes the account DID.
concept!: &member/account
  description: A member account of this space, by its DID.
  with:
    name: { the: xyz.tonk.member.account/name, as: text, … }

rule!:
  description: Every roster member is an account, named by its membership.
  assert: member/account
  when:
    - assert: member
      where: { member: ?this, name: ?name }

view!:
  this: member/account
  show:
    label: |
      {name}

palette/noun!:
  this: member/account
  name: "member"

palette/verb!:
  this: member/expel
  name: "expel"

palette/verb!:
  this: member/expel
  name: "remove member"

palette/argument!:
  command: member/expel
  field: expel-member/member
  role: palette/object
  noun: member/account
```

`tonk/rename-repository` is wired the same way (`"rename"`, object →
`tonk/repository`, goal → text).

## The FABB's commands (profile library)

What the FABB does through commands is sayable too: `tonk/pause-sync`
("pause sync", "resume sync") and the profile's `tonk/rename-repository`
("rename space"). Their noun is `space`, whose directory row is the
repository's own entity — the subject DID those commands take — so no rule
is needed.

`tonk/pause-sync` also carries `time`, a nonce the FABB fills from the
click's timestamp. Nothing typed should fill it, so its argument plays
`palette/now`: a role the palette fills with the moment the command runs
(the caller passes its clock in, so parsing stays pure) and never shows as
missing.

```yaml
palette/argument!:
  command: tonk/pause-sync
  field: pause-sync/time
  role: palette/now
```

Home is a command now too: `tonk/home {time}` ("home", "go home"), whose
handler posts a navigation to `/` back to the page that asked — the way a
join lands its tab on the new space. The FABB's other actions are not
commands: members, agent and account open panels, and share mints an invite
whose result the FABB itself copies. Opening a panel is page state, not a
fact; making those sayable wants a different answer (a page-side verb) than
a worker command.

## The pieces

| Piece | Where | What it does |
| --- | --- | --- |
| `dialog-palette` | dialog-db, `rust/dialog-palette` | The Parser 2 port: grammar, registry, 11-step pipeline, scoring, memory. Pure, no IO, native and wasm. |
| `tonk-palette` | tonk, `rust/tonk-palette` | Joins subscription rows into a registry (labels rendered with `tonk-template`), parses, builds the command claim. `web::install` puts `window.tonk.palette.parse` on the guest. |
| `<command-palette>` | `profile.yaml`, an `element!` (not `tonk-`: the element runtime never announces `tonk-` or `wa-` tags) | The FABB's command line: slotted into `<tonk-fab>`'s `command` slot, between the header and the actions, shown while the menu is open. With nothing typed the bar's actions show (the menu is the empty palette); typing sets `commanding` on the bar and the proposals take the actions' place. The top proposal completes inline as a selection (Tab/→ takes it, ↑/↓ or Ctrl+N/P move and the selection follows). Rows show the verb bold, things boxed with their kind (`space`, `member`), typed text quoted, and empty arguments dashed. On Cmd/Ctrl+K or Cmd/Ctrl+Shift+P it subscribes to the palette rows on `main@<space>` and the profile branch, resolves each noun concept the way `<tonk-display>` resolves a model (descriptor from `db.meta/source`, rows, `label` facet), calls `parse` per keystroke, and transacts the chosen claim. |

## Memory

Ubiquity's suggestion memory, as facts: every run asserts a
`palette/choice {command, input, time}` on the profile — one fact per run,
not a counter, so choices made on two devices both count after they sync.
The element subscribes to them and passes them to `parse`; `tonk-palette`
counts them per command (under the verb words typed, and overall), and the
parser raises a verb's score to the power `1/(1 + count)`, as Ubiquity did.
Nothing prunes them yet.

## Verified

- Natively (`tonk-worker` `router::palette`): a space seeded from
  `core.yaml`, read with the element's exact queries, parses "rename this
  to Q3", and the transacted claim renames the space through its rule.
- In the browser (dev server, headless Chromium, fresh profile): Ctrl+K in
  the space chrome opens the palette; "pause" offers "pause sync Welcome to
  Tonk", "rename space to Palette test" offers "rename space Welcome to Tonk
  to Palette test", and Enter renames the space — the FABB shows "Palette
  test". That rename is the profile's Rust-handled command, the one the
  FABB dispatches.
- In the browser: Ctrl+K inside the space's own frame opens the palette
  (the portal bridge relays the chord).
- In the browser: "go home" navigates the tab from `/space/…` to `/`; back
  in the space, the profile holds `palette/choice {command: tonk:home,
  input: "go home"}`.
- In the browser: "pause sync" writes `xyz.tonk.sync/enabled = false` on
  the space, "resume sync" writes `true`.

## Known defects

- **"pause sync" and "resume sync" are one toggle.** `tonk/pause-sync`
  flips the preference, so "resume sync" on a running space pauses it. The
  words promise a direction the command does not have. Either the command
  takes the desired state (an `enabled` field, with "pause" and "resume" as
  two verbs filling it differently — which the argument model cannot yet
  express, since a verb has no fixed field values), or the palette says
  only "toggle sync".
- **A name containing a delimiter reads two ways.** "rename Welcome to
  Tonk to Q3" is also "rename [Welcome…] to [Tonk to Q3]"; both score
  alike, so both are offered. Ubiquity has the same ambiguity; quoting, or
  preferring readings whose noun text is a whole label, would settle it.
- **No palette on the hub.** The element is mounted in the space chrome, so
  after "go home" there is nothing to open until a space is.

## What the proof of concept does not do yet

- **Existing spaces.** The palette rows ship in `core.yaml`, which seeds new
  spaces. Existing ones get them through the seed-upgrade path.
- **Notebook commands.** None is worth saying. `notebook/retitle` sets the
  title *from* the leading heading, so a palette retitle would leave title
  and heading disagreeing; `block/*` are the element's own plumbing
  (positions, chain pointers). A sayable notebook command ("add a heading",
  "rename notebook" that edits the heading) has to be designed first. If
  one is, the library lowers standalone and would redeclare `palette/*` the
  way the profile does — or the schema becomes built-in, as `event` did.
- **The active view.** `this` is the space. The entity the page shows (the
  notebook open in it) is not passed yet, so "rename this" cannot mean the
  notebook.
- **Preview.** A `preview` facet on the command, rendered with
  `tonk-render` against the parse's field values, is designed but not built.
- **Commands with fields no argument covers.** The claim carries only the
  arguments' fields; a command with another required field matches nothing.
  The proof-of-concept verbs cover every field of their commands.
