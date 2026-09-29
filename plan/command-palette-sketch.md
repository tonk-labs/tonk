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

## The pieces

| Piece | Where | What it does |
| --- | --- | --- |
| `dialog-palette` | dialog-db, `rust/dialog-palette` | The Parser 2 port: grammar, registry, 11-step pipeline, scoring, memory. Pure, no IO, native and wasm. |
| `tonk-palette` | tonk, `rust/tonk-palette` | Joins subscription rows into a registry (labels rendered with `tonk-template`), parses, builds the command claim. `web::install` puts `window.tonk.palette.parse` on the guest. |
| `<tonk-palette>` | `profile.yaml`, an `element!` | Mounted in the space chrome beside `<tonk-fab>`. On Cmd/Ctrl+K or Cmd/Ctrl+Shift+P it subscribes to the palette rows on `main@<space>`, resolves each noun concept the way `<tonk-display>` resolves a model (descriptor from `db.meta/source`, rows, `label` facet), calls `parse` per keystroke, and transacts the chosen claim. |

## What the proof of concept does not do yet

- **Keys inside the space frame.** The element listens on the chrome
  document; the space content is a nested sealed frame whose keys never reach
  it. A relay over the portal bridge is the fix.
- **Existing spaces.** The palette rows ship in `core.yaml`, which seeds new
  spaces. Existing ones get them through the seed-upgrade path.
- **Notebook and table commands.** Their libraries lower standalone, so they
  cannot reference `palette/*` from `core.yaml`. Either the schema becomes
  built-in (as `event` did, for the same reason), or each library declares it.
- **Memory.** Verb choices are not yet recorded as facts.
- **Preview.** A `preview` facet on the command, rendered with
  `tonk-render` against the parse's field values, is designed but not built.
- **Commands with fields no argument covers.** The claim carries only the
  arguments' fields; a command with another required field matches nothing.
  The proof-of-concept verbs cover every field of their commands.
