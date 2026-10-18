# Reactive views: events and effects

Templates with `{field}` interpolation cover the read path: a
view subscribes to a query and re-renders rows in place when the
data changes. Interactivity — clicks, form submits, anything a
user does — flows the other direction, and it is also fully
declarative.

The model has four pieces:

1. **`on:<name>=<command>`** on a template element binds an
   interaction: it names an event declaration (`on:click` names
   `on/click`) and the command to post when it fires.
2. An **event declaration** (`event!:`) says which platform event
   to listen for, whether to suppress its default handling, and
   where each command field's value comes from. The common ones
   (`on/click`, `on/submit`, `on/change`, …) are built in; declare
   your own when a command needs something they don't supply.
3. A **command** (`command!:`) describes the shape of the
   event-derived fact in domain terms. A command is a transient
   concept — `command!:` is equivalent to `concept!:` with
   `transient: true`. The command descriptor is durable schema; each
   event-derived instance of it lives one reactor cycle and is never
   persisted.
4. A **rule** (`rule!:`) watches for the transient and asserts
   downstream state. The transient is the trigger; its one-cycle
   lifetime makes the rule fire exactly once per event.

No JavaScript. No `fetch`. No `globalThis.tonk`. The bridge into
the worker is the transient assertion, posted by the runtime
when the event fires; you describe the projection, not the wire
call.

### Checkbox state is separate from the change event

For a boolean `done` field, use `checked={done}` to render stored state,
for example `<input type="checkbox" checked={done} on:toggle=toggle-task>`.
Define `toggle-task` before using that binding. The built-in `on/toggle`
declaration fills a `subject` field with the row's entity and a `checked`
field with the box's new boolean, so the command declares exactly those two
fields (`subject` as `entity`, `checked` as `boolean`); a rule must persist
the new boolean on the existing task. Give each command its own attribute
namespace when multiple commands could share a shape (see below).

A bare `{done}` inside the tag does not initialize or update the checkbox's
checked property. A visual click alone proves neither persistence nor sync.
Read back the record, reload, and check a second client before claiming both.
See the views guide for the directory and home-page recipe.

### Wiring a click

```yaml tonk=eval
attribute!: &count
  description: The counter's current value
  the: xyz.tonk.counter/count
  as:  natural

concept!: &counter
  with:
    count: count

# The event-derived command — a transient concept the reactor
# sweeps after one cycle instead of persisting. Its field is
# domain-shaped; the built-in `on/click` declaration fills
# `subject` with the entity the view is rendering.
command!: &increment
  with:
    subject:
      description: The counter entity to increment
      the: xyz.tonk.counter.increment/subject
      as:  entity

rule!:
  assert!: counter
  when:
    - assert: increment
      where: { subject: ?this }
    - assert: counter
      where: { this: ?this, count: ?old }
    - assert: math/sum
      where: { of: ?old, with: 1, is: ?count }

view!:
  this: counter
  show:
    ui: |
      <p>{count}
        <button on:click=increment>+</button>
      </p>
```

The view lives ON the model: `this: counter` is the concept being
rendered, and `ui` is the `show` facet `<tonk-display>` picks by
default. Re-asserting the facet supersedes the template. `tonk view
add counter --template …` authors this shape for you.

A click on the button fires `on/click`, which is declared as:

```yaml tonk=illustrative-builtin
event!: &on/click
  type: "click"
  where:
    subject: "{this}"
```

So the click asserts an `increment` whose `subject` is `{this}` —
the counter the button was rendered for. No `data-*` attribute is
needed: `{this}` is the subject of the enclosing row (or, in a
single-entity view, the entity the `<tonk-display>` shows). The
rule reads the `increment` together with the current `counter`,
computes the new total, and asserts the updated `counter`. The
subscription behind the view sees the new count and patches the
DOM. The `increment` fact is gone next cycle — it never reaches
storage.

### `on:<name>=<command>` syntax

- The attribute name picks the declaration: `on:click` binds
  `on/click`, `on:todo-move` binds `on/todo-move`. (`/` is not
  legal in an HTML attribute name, `:` is, so they differ by
  exactly that substitution.) Only the `on:` prefix is reserved;
  `xlink:href` and friends are untouched.
- The value is the command: a name (`increment`, `todo/complete`)
  or an entity URI (anything containing `:`, e.g.
  `tonk:invite`). Quoting is optional: `on:click="todo/complete"`.
- The platform event type lives on the declaration, not the
  attribute name, so two declarations can read the same platform
  event differently (`on/click` and `on/click-once` both listen
  for `click`).
- One element may carry several bindings for different
  declarations (`on:click=… on:keydown-enter=…`). When nested
  elements both bind the same event, the innermost binding that
  resolves wins; one that cannot fill its command falls through
  to the next bound ancestor.
- A binding with an empty value is ignored.
- `tonk eval` checks every binding when it installs the view: a
  declaration that resolves to nothing is
  `E_UNKNOWN_EVENT_DECLARATION`, a command that resolves to
  nothing is `E_UNKNOWN_BOUND_COMMAND`. When the declaration is in
  the same document, a declaration that cannot fill its command
  is `E_EVENT_COMMAND_MISMATCH` and a `{field}` source the view's
  model doesn't declare is `E_UNKNOWN_EVENT_SOURCE_FIELD`.

### Built-in declarations

| Attribute     | Listens for | Fills                                                        | Side effect        |
|---------------|-------------|--------------------------------------------------------------|--------------------|
| `on:click`      | `click`     | `subject` ← `{this}`                                         |                    |
| `on:click-once` | `click`     | `subject` ← `{this}`, `time` ← `.timeStamp`                  |                    |
| `on:submit`     | `submit`    | `subject` ← `{this}`                                         | `prevent-default`  |
| `on:change`     | `change`    | `subject` ← `{this}`, `value` ← `.currentTarget.value`       |                    |
| `on:toggle`     | `change`    | `subject` ← `{this}`, `checked` ← `.currentTarget.checked`   |                    |

**A command bound to a declaration must declare exactly the
fields it fills.** Every `with:` field needs a source, and every
source must name a field the command has (`with:` or `maybe:`).
For the built-ins this means naming your command's fields
`subject`, `value`, `checked` and `time` as the table shows —
`time` as a `float`, since `.timeStamp` is a number. A built-in
declaration lives on the branch, not in your document, so `tonk
eval` cannot check the fit for you: a command whose field is
called `counter` instead of `subject` installs cleanly, posts
without it, and the rule never fires. When a command needs
anything else, declare your own event in the same document,
where the analyzer does check it.

### Declaring your own event

```yaml tonk=illustrative-shape
event!: &on/<name>
  type: "<platform event>"   # click, submit, change, keydown, or a CustomEvent name
  prevent-default: true      # optional; call event.preventDefault()
  stop-propagation: true     # optional; call event.stopPropagation()
  where:
    <command field>: <source>
```

Each `where:` source is one of these forms. Write the first three
quoted — a bare `{this}` is a YAML mapping, not a string:

| Source               | reads                                                                 |
|----------------------|-----------------------------------------------------------------------|
| `"{this}"`           | the subject of the enclosing row (or of the `<tonk-display>`)         |
| `"{name}"`           | the `data-name` attribute on the bound element or its nearest ancestor |
| `".key"`             | `event.key`                                                           |
| `".timeStamp"`       | `event.timeStamp`                                                     |
| `".currentTarget.value"` | the bound element's `value`                                       |
| `".currentTarget.dataset.todo"` | the bound element's `data-todo`                            |
| `".currentTarget.elements.body.value"` | a form's `body` control, when the form is bound       |
| `".detail.sheet"`    | a CustomEvent's `event.detail.sheet`                                  |
| `"Untitled"`         | the constant text                                                     |
| `inbox`, `did:key:…` | the entity the name refers to, or the entity URI itself               |

Rules of the translation:

- A `.path` is a chain of property reads on the live event,
  spelled exactly the way JavaScript spells it (`.shiftKey`,
  `.currentTarget.dataset.itemId`). Nothing is case-converted, so
  a control named `note-body` is read as
  `.currentTarget.elements.note-body.value`.
- `.currentTarget` is the **bound element**: the element carrying
  the `on:` attribute that matched. (Not the literal
  `event.currentTarget`, which is the host where the delegated
  listener lives.) `.target` is the *deepest* node the event fired
  on, which may be a child you didn't author (e.g. a `<span>`
  inside the button), so prefer `.currentTarget`.
- `.detail` is the payload a custom element dispatches with a
  `CustomEvent` (e.g. a `<tonk-sheet-binder>` emitting
  `{detail: {sheet}}`). Plain DOM events have no `detail`; this is
  how you read fields off an event a component raised rather than
  a raw click.
- `"{name}"` interpolates in the scope of the element the event
  fired on. `{this}` needs no markup; any other `{name}` reads a
  `data-name` attribute the template must render
  (`data-title={title}`).
- The path is evaluated live, at event time. A source that fails
  to resolve — a missing/`undefined` step (e.g. `.pressure` on a
  `MouseEvent`), or a value that won't coerce to the command
  field's `as:` — means the binding does not apply: nothing is
  posted, nothing is logged, and the event falls through to the
  next bound ancestor.
- A **blank** leaf (`""` from an empty text input, `null` from a
  blank `<wa-input>`) is "not provided": the field is **omitted** and
  the command still posts without it. No error is logged — but a rule
  premise that names the missing field then matches nothing, so the
  event silently does nothing. Read as little as possible from the
  form and derive the rest in the rule from branch data; if a field
  can legitimately be empty, declare it under the command's `maybe:`
  and leave it out of the rule premise, or send a sentinel from the
  component (e.g. `<br>` for empty rich text) rather than `""`.
- The command field's `as:` type drives coercion: `text` → string,
  `entity` → string parsed as a URI (rejected if it has no `:`),
  `natural` / `integer` / `float` → the JS value must already be a
  number (an input's `.value` is a string, so read it as `text`),
  `boolean` → the JS value must already be a boolean (e.g.
  `.shiftKey`, `.currentTarget.checked`).

### One command, one shape — give each command its own attributes

Commands are matched **structurally**: a rule premise `assert:
<command>` matches ANY transient carrying all of that command's
attributes — not just transients posted under that command's name. So
two commands built from the same attributes are the same shape, and
one event fires both rules. The analyzer rejects different named
transient commands used as positive rule triggers when their required
attribute sets are equal or one is a subset of the other:

```yaml tonk=illustrative-overlap
# WRONG: same attribute — an "activate" event also fires the delete rule.
attribute!: &page/target
  description: The page being acted on
  the: xyz.tonk.page/target
  as: entity
command!: &activate-page
  with:
    page: page/target
command!: &delete-page
  with:
    page: page/target
```

Give each command verb-specific attributes — its own namespace —
even when the field *names* match:

```yaml tonk=eval
command!: &activate-page
  with:
    page: { description: The page to activate, the: xyz.tonk.page.activate/page, as: entity }
command!: &delete-page
  with:
    page: { description: The page to delete, the: xyz.tonk.page.delete/page, as: entity }
```

Field names are local to a command, so both can be called `page`
(or `subject`) and both can be filled by the same declaration;
only the `the:` attributes need to differ.

### Passing view context

A binding inside a repeat row already knows its row: `{this}` is
that row's subject. Reach for `data-*` only when the command needs
something other than the row itself — a parent, a sibling list, a
constant per button — and read it back in your own declaration:

```yaml tonk=parse
event!: &on/todo-move
  type: "click"
  where:
    subject: "{this}"
    list: ".currentTarget.dataset.list"

command!: &todo/move
  with:
    subject: { description: The todo to move, the: xyz.tonk.todo.move/subject, as: entity }
    list: { description: The list to move it to, the: xyz.tonk.todo.move/list, as: entity }

view!:
  this: todo
  show:
    ui: |
      <li>
        <span>{title}</span>
        <button on:todo-move=todo/move data-list={list}>move</button>
      </li>
```

The chain `data-list={list}` → the bound button's `data-list` →
`.currentTarget.dataset.list` → the command's `list` field is
visible at every step. `.currentTarget` reads the `data-*` off the
element the binding was authored on, so a click that lands on a
child node still resolves. The template author decides what to
surface; the declaration decides what to read; both sides have to
agree on the `data-<name>`. (`"{list}"` would read the same
`data-list` attribute.)

### Side effects: `preventDefault` and friends

`preventDefault()` and `stopPropagation()` are method calls, not
values. They have to run synchronously inside the event handler —
by the time the worker responds to the assertion, the event is
gone. Declare them as flags on the **event declaration**:

| Flag                     | calls                     |
|--------------------------|---------------------------|
| `prevent-default: true`  | `event.preventDefault()`  |
| `stop-propagation: true` | `event.stopPropagation()` |

They run only for the binding that wins, and only once every source
resolved. Because they live on the declaration, the command keeps
nothing but domain fields, so a rule can consume a form-submit
command like any other:

```yaml tonk=eval
attribute!: &note/body
  description: A note's text
  the: xyz.tonk.note/body
  as: text

concept!: &note
  with:
    body: note/body

event!: &on/note-save
  type: "submit"
  prevent-default: true
  where:
    subject: "{this}"
    body: ".currentTarget.elements.body.value"

command!: &note/save
  with:
    subject: { description: The note to save, the: xyz.tonk.note.save/subject, as: entity }
    body: { description: The note body to save, the: xyz.tonk.note.save/body, as: text }

rule!:
  assert!: note
  when:
    - assert: note/save
      where: { subject: ?this, body: ?body }

view!:
  this: note
  show:
    ui: |
      <form on:note-save=note/save>
        <textarea name="body">{body}</textarea>
        <button type="submit">save</button>
      </form>
```

The submit fires on the `<form>`, so `.currentTarget` is the form
and `elements.body` is the control named `body` — its `value`
becomes the `body` field. `prevent-default` stops the page from
reloading. When a form only needs the row's subject, the built-in
`on:submit` already declares `prevent-default: true`.

### `rule!:` — the reactive layer

A rule has a head with one polarity and a body of premises:

```yaml tonk=illustrative-placeholders
rule!:
  assert!: <head-concept>      # or `retract!: <head-concept>`
  when:
    - assert: <concept>
      where: { <field>: ?var, ... }
    - ...
  unless:                       # optional negative premises
    - assert: <concept>
      where: { ... }
```

Three things to know:

- **Head fields come from variable names in the body.** If the
  head is `counter` (fields `this`, `count`) and the body binds
  `?this` and `?count`, the head asserts `counter{this: ?this,
  count: ?count}`. No explicit `where:` on the head.
- **At least one positive `when:` premise must read a transient
  concept.** This is the trigger. A rule with only persistent
  premises is rejected at install time (the reactor can't tell
  when it should fire). If you find yourself wanting a rule
  without a trigger, you usually want a deductive query
  instead.
- **`assert!:` produces new facts; `retract!:` removes them.**
  Multiple rules can share a head and compose by disjunction —
  adding a new event means adding a new rule, not editing
  existing ones.

Built-in formulas like `math/sum` (`{of: ?a, with: ?b, is:
?c}`) and comparisons participate as premises, threading values
between bindings.

### Retracting from a rule

To remove a fact in response to a transient, use `retract!:`:

```yaml tonk=parse
command!: &complete
  with:
    subject:
      description: The todo to complete
      the: xyz.tonk.todo.complete/subject
      as:  entity

rule!:
  retract!: todo
  when:
    - assert: complete
      where: { subject: ?this }
```

Bound with `<button on:click=complete>done</button>` inside the
todo's row, the click-derived `complete` transient carries the
row's subject, and the rule retracts the matched `todo`. The
view's subscription sees the row disappear.

### Minting a new entity from a rule (create forms)

A rule head needs every field bound, **including `this`** — there is
no formula that generates a fresh entity. To create durable state
from an event (a "new post" form), bind the transient command's own
content-derived entity as the new fact's identity:

```yaml tonk=parse
event!: &on/post-publish
  type: "submit"
  prevent-default: true
  where:
    body: ".currentTarget.elements.body.value"
    time: ".timeStamp"

command!: &publish
  with:
    body: { description: The post body to publish, the: xyz.tonk.post.publish/body, as: text }
    time: { description: When it was published, the: xyz.tonk.post.publish/time, as: float }

rule!:
  assert!: post
  when:
    - assert: publish
      where: { this: ?this, body: ?body }   # the transient's own entity
```

```yaml tonk=parse
view!:
  this: board
  show:
    ui: |
      <form on:post-publish=publish>
        <textarea name="body"></textarea>
        <button type="submit">Post</button>
      </form>
```

One durable entity per event, carrying the command's identity. A
command's entity is content-derived, so two events with identical
parameters would collide on the same entity; the `time` field
(`.timeStamp`) makes each event's body — and therefore its entity —
differ. The built-in `on:click-once` does the same for a button.

Everything the head asserts must be bound in the body: read what the
user typed from the form, and pull the rest (author, defaults) from
persistent facts joined in additional `when:` premises, or bind
constants with `==` / formula premises.

### Debugging: the click did nothing

The failure modes, from loud to silent:

1. **`tonk eval` rejects the view** — `E_UNKNOWN_EVENT_DECLARATION`
   (no `on/<name>` exists), `E_UNKNOWN_BOUND_COMMAND` (the command
   name resolves to nothing), `E_EVENT_COMMAND_MISMATCH` (a
   same-document declaration leaves a command field unfilled, or
   fills one the command doesn't have), or
   `E_OVERLAPPING_TRANSIENT_COMMANDS` (see "One command, one
   shape"). The message names the attribute; fix what it says.
2. **The command's fields don't match a built-in declaration** —
   the analyzer can't check a declaration that lives on the branch,
   so a command field named anything other than what the
   declaration fills (see the built-in table) is never set, and
   the rule's premise matches nothing. Rename the field or declare
   your own event in the same document.
3. **A source didn't resolve** — a misspelled or missing property
   step, a `data-*` the template didn't render, or a value that
   won't coerce to `as:` (an `entity` needs a `:`, a number field
   can't take an input's string `.value`). The binding silently
   doesn't apply; nothing posts.
4. **The command posted but the rule didn't fire** — a form
   control resolved blank (`""`/`null`), so the field was omitted
   and the posted command doesn't satisfy the rule's premise. See
   the blank-leaf bullet above.
5. **The wrong rule fired (or two did)** — commands match
   structurally, not by name; see "One command, one shape".
6. **The form reloaded the page** — the binding didn't apply
   (cases 2–3), so its `prevent-default` never ran and the native
   submit won.

To isolate a rule from the DOM wiring, assert the command's shape
directly with `tonk eval` and check the rule's output fact appears.
Caveat: notation forces a value into every field — including ones the
real DOM event can't produce (a blank input, a field no declaration
fills) — so a rule that fires under eval can still be unreachable from
the DOM; cases 2 and 4 are exactly that false positive.

### Opening the view in tonk-ui

Events fire in the live shell, not in a standalone page, so open the
view in the space: `/space/<space>/<model>` for the model's directory,
`/space/<space>/<entity>@<model>!<view>` for one entity through a
named view concept. `tonk render` is no substitute here — it prints the
HTML with nothing behind it, so a click reaches no command. Use it to
check the markup, the shell to check the wiring.

For a raw first build, `tonk eval interactive.notation --home todo`
installs the rules and views and replaces the home with `todo` in one
transaction. Use `tonk space home todo` when repointing the home later.

To put a collaborator in front of the same view, hand them the repo
with `tonk invite`.

### When the declarative path doesn't fit

Interactions a template can't express — caret management, drag
and drop, rich text editing — belong in a **web component** that
dispatches `CustomEvent`s, bound with your own `event!:` whose
`type` is the custom event's name and whose sources read
`.detail.*`, exactly like the built-in `<tonk-sheet-binder>`.
Components are authored as branch data (the `component` concept)
and stay inside the concept-and-rule pipeline; see `tonk help
views`.

Anything that instead needs a whole isolated page — third-party
embeds, self-contained canvas apps — belongs in a sandboxed
iframe view (`<tonk-portal>`), outside the scope of this guide.

---

Don't memorize concept fields — use `tonk concept` and
`tonk show <concept>`. Rules themselves remain notation documents;
keep the source that installed them under version control.
