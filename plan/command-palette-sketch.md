# Command palette: a Ubiquity sketch in dialog

Status: sketch. `phrase!` and `suggest!` below are **hypothetical syntax**.
`command!`, `concept!`, `view!` and `rule!` are the library's real shapes.
Companion to [command-palette.md](command-palette.md). Where the two disagree,
this document is the newer model.

There is no new vocabulary of verbs and nouns. **Commands and concepts stay the
vocabulary**, and each gets one more declaration beside it, the way `event!`
sits beside a command today:

- **`suggest!`** says how a concept's instances are offered and matched. It
  covers what Ubiquity's noun type did.
- **`phrase!`** says how text parses into a command's fields. It covers what
  Ubiquity's verb grammar did.

## Why this shape and not separate verbs

An earlier draft made verbs their own transient concepts, with rules bridging
them to commands. Checking the worker killed that:

- `tonk-worker/src/router/transact.rs` snapshots `builder.transients` **before**
  commit and hands only that snapshot to `dispatch`.
- So a Rust provider sees the transients the page asserted, never the ones a
  rule emits during induction. Dialog chains transients between rounds
  (`MAX_ROUNDS` in `dialog-repository/.../transaction/induce.rs`), but tonk's
  `CommandRegistry` is outside that loop.
- A verb → `tonk/rename-repository` bridge rule would therefore commit and run
  nothing, which is the silent-success failure again.

A `phrase!` makes the palette assert **the command itself**, exactly as an
`event!` does. Everything that already handles a command keeps working, and
nothing new sits between the palette and the handler.

## What a Ubiquity command was, and where it goes

```js
CmdUtils.CreateCommand({
  names: ["translate"],
  arguments: [
    { role: "object", nountype: noun_arb_text,         label: "text" },
    { role: "goal",   nountype: noun_type_lang_google, label: "language" },
  ],
  preview(pblock, args) { /* … */ },
  execute(args)         { /* … */ },
});
var noun_type_lang_google = { label: "language", suggest(text) { /* … */ } };
```

| Ubiquity | Here |
| --- | --- |
| a noun type and its `suggest(text)` | `suggest!` on a concept: which fields text matches, and the `label` view |
| `names` plus `arguments` with roles and markers | `phrase!` templates such as `"rename {thing} to {name}"` |
| `execute(args)` | the command the phrase fills, asserted as a transient |
| `preview(pblock, args)` | a `preview` view facet on the command concept |
| the selection | `{this}`, from the `tonk:site` entity |
| the registry | a dialog subscription over `phrase` and `suggest` |

Roles and markers drop out. A template spells its own connecting words, and
localizing means adding templates for another locale. That is less clever
than Ubiquity Parser 2, and much easier to author and predict.

## `suggest!`: a concept as a source of candidates

```yaml
suggest!:
  concept: tonk:member
  match: [name]                  # stored text fields the typed text is matched against
  # label: `show: label` on tonk:member, falling back to the first `match` field

suggest!:
  concept: notebook/named
  match: [title]

suggest!:
  concept: space
  match: [name]
  where:                         # scope: never offer the profile's self-replica
    kind: tonk:repository
```

- **Candidates** are `Query<concept>` constrained by `where:`. Because `match`
  names *stored* fields, the typed prefix is pushed down with `StartsWith` into
  an index range. The survivors are fuzzy-ranked in Rust.
- **Display** is the concept's `label` view, the facet that already exists
  (`tonk:repository` → `{name}`).
- **Aliases** ("me" for the signed-in member) are ordinary facts or rules that
  add a matchable field, for example `alias`, and are listed in `match`.
- A concept without `suggest!` can still be named by a phrase slot. It just
  offers no completions, only `{this}` or a pasted identifier.

## `phrase!`: text into a command's fields

```yaml
phrase!:
  command: notebook/retitle
  say:
    - "rename {thing} to {title}"
    - "retitle {thing} {title}"
  slots:
    thing: notebook/named        # a concept with suggest!
    title: text                  # a value type
  default:
    thing: "{this}"              # used when the slot is empty and {this} conforms
  where:                         # command field ← slot or context, same mini-language as event!
    subject: "{thing}"
    title: "{title}"

phrase!:
  command: tonk/rename-repository
  say:
    - "rename {thing} to {name}"
  slots:
    thing: space
    name: text
  default:
    thing: "{space}"
  where:
    space: "{thing.subject}"     # project the candidate to the field the command wants
    name: "{name}"
```

- **One sentence can reach several commands.** Both phrases above say "rename
  {thing} to …". The parser tries both, and **the slot's concept decides**: a
  notebook matches the first, a space the second. Ubiquity did this with
  `if` branches in `execute`; here it falls out of noun matching.
- **`where:` is `event!`'s mapping, retargeted.** `event!` reads
  `.currentTarget.dataset.x`, while `phrase!` reads `{slot}`,
  `{slot.field}`, `{this}`, `{space}`, `{profile}` and `{now}`. This is also where
  the command's handler-shaped fields (a DID, a timestamp nonce) get filled
  without the person ever typing them.
- **Commands with no phrase are simply not sayable.** `block/place`,
  `tonk/load` and the rest of the plumbing need nothing.

### Guards and consequences

```yaml
phrase!:
  command: member/expel
  say: ["expel {member}", "remove {member}", "kick {member}"]
  slots:
    member: tonk:member
  where:
    member: "{member.member}"
  when:                          # only offered to admins
    - assert: tonk/member-role
      where: { member: "{profile}", role: tonk:founder }
  destructive: true              # asks before running

view!:
  this: member/expel
  show:
    preview: |
      remove <tonk-display entity={member} model=tonk:member view=label></tonk-display>
      from this space
```

`tonk/member-role` is a placeholder for whatever the roster exposes. The point
is that applicability is a premise the palette queries, not code in the
palette.

**Preview is a view on the command.** A command is a concept, so it can have a
`preview` facet like any model. The palette renders it over the candidate
command placed in the query overlay, so nothing is committed. A command with
no `preview` shows its phrase with the slots filled by their labels.

### Things without a command

- **Going somewhere.** Add one command, `tonk/open {target}`, with a
  `Provider` that resolves the target's route and posts a navigate to the
  originating client, as `tonk:join` does. Then each openable concept is just a
  phrase:

  ```yaml
  phrase!:
    command: tonk/open
    say: ["open {notebook}", "go to {notebook}", "{notebook}"]
    slots: { notebook: notebook/named }
    where: { target: "{notebook}" }
  ```

  The bare `"{notebook}"` template is what makes noun-first work: typing a
  notebook's title offers opening it.
- **Finding.** "find roadmap" is a `suggest!` whose `match` covers content, not
  just titles, reached through the same `open` phrase. A real preview-only
  phrase (one that shows a query result and runs nothing) is a later addition:
  `phrase!` with `shows:` a concept instead of `command:`.

## Parsing, walked through

The palette opened on notebook `N` ("Roadmap") in space `S`.

**`rename this to Q3 plan`**

1. Two templates match the literal words `rename … to …`. The holes capture
   `thing` = "this" and the second slot = "Q3 plan".
2. `this` resolves through `{this}` = `N`.
   - The first phrase's `thing` is `notebook/named`, and `N` conforms, which is
     one point query.
   - The second phrase's `thing` is `space`, and `N` doesn't conform, so that
     candidate drops.
3. The candidate is `notebook/retitle {subject: N, title: "Q3 plan"}`.
4. Preview renders over the overlay. Enter asserts the command transient,
   exactly as `<tonk-notebook>`'s `titlechange` event does today.

**`ren`**

1. A fuzzy prefix hit on the literal word of both rename templates.
2. `thing` defaults to `{this}`, so only the notebook phrase stays complete
   enough to show: "rename Roadmap to …".
3. Enter steps into `title` as a prompt.

**`alice`**

1. No literal words match.
2. Bare-slot templates (`"{member}"`, `"{notebook}"`) try the text as a noun.
   It hits a `tonk:member` row, "Alice".
3. Rows are offered for every phrase whose slot accepts that concept: "open
   Alice", and "expel Alice" if you are an admin.

**`rename to do list to done`**

1. The literal `to` appears twice, so there are two splits:
   - `thing` = "", with the rest = "do list to done";
   - `thing` = "to do list", with the rest = "done".
2. The first split leaves `thing` to default to `{this}`. The second finds a
   notebook titled "to do list".
3. A slot that matched an entity outranks a defaulted one, so the second
   split wins. Ambiguity is resolved by noun matching, not by grammar.

## What the Rust side sees

```rust
pub struct Registry {
    phrases: Vec<Phrase>,          // from `phrase` rows on every layer
    suggest: HashMap<Entity, Suggest>, // concept → how to offer it
}

pub struct Phrase {
    command: CommandDescriptor,    // what gets asserted, inline, as the FAB does
    templates: Vec<Template>,      // literal words + holes
    slots: HashMap<String, Slot>,
    defaults: HashMap<String, Binding>,
    fill: HashMap<String, Binding>,// `where:`: command field ← slot/context path
    applies: Option<Premises>,     // `when:`
    destructive: bool,
}

pub enum Slot { Value(Type), Concept(Entity) }

pub struct Suggest { concept: Entity, matches: Vec<Attribute>, filter: Parameters }

pub trait Nouns {                  // over a dialog session; faked in native tests
    async fn suggest(&self, s: &Suggest, text: &str, limit: usize) -> Vec<Candidate>;
    async fn conforms(&self, entity: &Entity, concept: &Entity) -> bool;
}
```

`parse(&Registry, &Context, text) -> Vec<Candidate>` is pure except for
`Nouns`. A smarter ranker (embeddings, a local model) plugs in where the
literal words are matched, and nothing in the declarations changes.

## Frozen seeds

`phrase!` and `suggest!` are separate declarations keyed by name, like
`event!`. They never touch a command's or concept's identity. Tonk's own phrases
ship in the built-in layer, and the palette builds each claim with an inline
command descriptor, the way the FAB's `*_claim_json` builders do and
`fab_drift.rs` guards. That keeps them working on profiles and spaces seeded
long before the palette existed.

## Still open

- **Rendering a preview over an overlay.** `<tonk-display>` renders against a
  branch. Branch ⊕ one uncommitted command is a new mode for the display, even
  though the query layer already has it.
- **Context in the FABB frame.** `{this}` lives on the space-side `tonk:site`
  entity, which the FABB frame can't see today. It travels with the key relay.
- **Rule-emitted transients never reach Rust providers.** The palette design no
  longer depends on this, but it is a real gap: any `rule!` that asserts a
  Rust-handled command silently does nothing. It deserves its own issue.
- **Template ambiguity cost.** Each repeated literal multiplies the splits.
  Cap the splits per template and measure with real phrase sets.
