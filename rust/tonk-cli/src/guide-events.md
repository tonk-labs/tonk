# Reactive views: events and effects

Templates with `{field}` interpolation cover the read path: a
view subscribes to a query and re-renders rows in place when the
data changes. Interactivity — clicks, form submits, anything a
user does — flows the other direction, and it is also fully
declarative.

The model has three pieces:

1. **`on<event>=<concept>`** on a template element names the
   concept a DOM event produces.
2. A **command** (`command!:`) describes the shape of that event-
   derived fact and where each field reads from. A command is a
   transient concept — `command!:` is equivalent to `concept!:` with
   `transient: true`. The command descriptor is durable schema; each
   event-derived instance of it lives one reactor cycle and is never
   persisted.
3. A **rule** (`rule!:`) watches for the transient and asserts
   downstream state. The transient is the trigger; its one-cycle
   lifetime makes the rule fire exactly once per event.

No JavaScript. No `fetch`. No `globalThis.tonk`. The bridge into
the worker is the transient assertion, posted by the runtime
when the event fires; you describe the projection, not the wire
call.

### Wiring a click

```yaml tonk=eval
attribute!: &count
  description: The counter's current value
  the: xyz.tonk.counter/count
  as:  unsigned-integer

concept!: &counter
  with:
    count: count

# The event-derived command — a transient concept the reactor
# sweeps after one cycle instead of persisting.
command!: &increment
  with:
    subject:
      description: The counter entity to increment
      the: dom.event.current-target.dataset/subject
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
        <button onclick=increment data-subject={this}>+</button>
      </p>
```

The view lives ON the model: `this: counter` is the concept being
rendered, and `ui` is the `show` facet `<tonk-display>` picks by
default. Re-asserting the facet supersedes the template. `tonk view
add counter --template …` authors this shape for you.

A click on the button asserts an `increment` whose `subject`
reads from the bound button's `data-subject` (which the template
populated from the row's `{this}`). The rule reads the
`increment` together with the current `counter`, computes the
new total, and asserts the updated `counter`. The subscription
behind the view sees the new count and patches the DOM. The
`increment` fact is gone next cycle — it never reaches storage.

### `on<event>=<concept>` syntax

- One event attribute per event type per element. Two clicks
  that should produce two facts means two elements.
- The right-hand side is a concept name (`increment`) or an
  entity URI (anything containing `:`, e.g.
  `did:key:zHj…`). Both go through a phase-1 descriptor
  lookup — a name resolves through the branch's name table, a
  URI resolves directly by entity.
- A bare `onclick` with no concept is ignored: it resolves to
  no descriptor, so the click is a silent no-op rather than an
  error.
- Any DOM event name works: `onclick`, `onsubmit`,
  `onkeydown`, `onpointerdown`. The runtime registers a
  listener for the literal type.

### The `dom.event` namespace

A transient concept's attributes describe what to read out of
the live JS event. The `the:` identifier names a path:

| `the:` identifier                             | reads                                |
|-----------------------------------------------|--------------------------------------|
| `dom.event/type`                              | `event.type`                         |
| `dom.event/key`                               | `event.key`                          |
| `dom.event/shift-key`                         | `event.shiftKey`                     |
| `dom.event/client-x`                          | `event.clientX`                      |
| `dom.event.target/value`                      | `event.target.value`                 |
| `dom.event.current-target.dataset/subject`    | the bound element's `data-subject`   |
| `dom.event.current-target.dataset/item-id`    | the bound element's `data-item-id`   |
| `dom.event.detail/sheet`                      | a CustomEvent's `event.detail.sheet` |

Rules of the translation:

- Split the identifier on `.` and `/`. Each segment is a
  successive property step on the event object.
- Within a segment, kebab-case becomes camelCase before lookup
  (`shift-key` → `shiftKey`, `item-id` → `itemId`).
- A leading `detail` segment is `event.detail` — the payload a
  custom element dispatches with a `CustomEvent` (e.g. a
  `<tonk-sheet-binder>` emitting `{detail: {sheet}}`). Plain DOM
  events have no `detail`; this is how you read fields off an
  event a component raised rather than a raw click.
- A leading `target` segment is `event.target` — the *deepest*
  node the event fired on. A leading `current-target` segment
  is the **bound element**: the closest ancestor of
  `event.target` that carries the `on<event>` you authored.
  (This is not the literal `event.currentTarget`, which points
  at the host where the delegated listener lives.) To read a
  `data-*` you put on the element with the handler, always use
  `current-target` — `target` may be a child you didn't author
  (e.g. a `<span>` inside the button).
- The path is evaluated live, at handler-fire time. A path that
  fails to resolve — a missing/`undefined` step (e.g.
  `event.pressure` on a `MouseEvent`), or a value that won't
  coerce to `as:` — aborts the *whole* assertion: nothing is
  posted and the event falls through to the next matching
  binding. Only a `the:` that doesn't address the event at all
  is silently dropped.
- A **blank** leaf (`""` from an empty text input, `null` from a
  blank `<wa-input>`) is "not provided": the field is **omitted** and
  the command still posts without it. No error is logged — but a rule
  premise that names the missing field then matches nothing, so the
  event silently does nothing. Read as little as possible from the
  form and derive the rest in the rule from branch data; if a field
  can legitimately be empty, send a sentinel from the component
  (e.g. `<br>` for empty rich text) rather than `""`.
- The `as:` type drives coercion: `text` → string, `entity` →
  string parsed as a URI (rejected if it has no `:`),
  `unsigned-integer` / `signed-integer` / `float` → number,
  `boolean` → the JS value must already be a boolean (e.g.
  `event.shiftKey`); a non-boolean fails to resolve rather than
  being coerced.

### One command, one shape — don't share detail attributes

Commands are matched **structurally**: a rule premise `assert:
<command>` matches ANY transient carrying all of that command's
attributes — not just transients posted under that command's name. So
two commands that read the same event attributes are the same shape,
and one event fires both rules. The analyzer rejects different named
transient commands used as positive rule triggers when their required
descriptor sets are equal or one is a subset of the other:

```yaml tonk=illustrative-overlap
# WRONG: same shape — a "navigate" event also fires the delete rule.
command!: &activate-page
  with:
    page: { description: The page to activate, the: dom.event.detail/page, as: entity }
command!: &delete-page
  with:
    page: { description: The page to delete, the: dom.event.detail/page, as: entity }
```

Give each command verb-specific attributes, and make sure no
command's attribute set is a subset of what another event carries:

```yaml tonk=eval
command!: &activate-page
  with:
    page: { description: The page to activate, the: dom.event.detail/activate, as: entity }
command!: &delete-page
  with:
    page: { description: The page to delete, the: dom.event.detail/delete, as: entity }
```

(The event *detail* keys follow: `detail: { activate: <uri> }` vs
`detail: { delete: <uri> }`.) The built-in sheets binder does exactly
this — `dom.event.detail/sheet` for activation, a separate
`dom.event.detail/closed` for closing.

### Passing view context via `data-*`

A click handler doesn't implicitly know the row it was rendered
for. Surface what you need on the element with `data-*` and
read it back through `dom.event.current-target.dataset/<name>`:

```yaml tonk=parse
view!:
  this: todo
  show:
    ui: |
      <li>
        <span>{title}</span>
        <button onclick=complete data-todo={this}>done</button>
      </li>

command!: &complete
  with:
    todo:
      description: The todo to complete
      the: dom.event.current-target.dataset/todo
      as:  entity
```

The chain `data-todo={this}` → the bound button's
`data-todo` → `?todo` is visible at every step. `current-target`
(not `target`) reads the `data-*` off the element the handler
was authored on, so a click that lands on a child node — say a
`<span>` inside the button — still resolves. The template author
decides what to surface; the concept author decides what to
read; both sides have to agree on the `data-<name>`.

### Side effects: `preventDefault` and friends

`preventDefault()` and `stopPropagation()` are method calls,
not values. They have to run synchronously inside the event
handler — by the time the worker responds to the assertion,
the event is gone. Declare them as attributes under the
`dom.event.do` namespace:

| `the:` identifier                          | calls                              |
|--------------------------------------------|------------------------------------|
| `dom.event.do/prevent-default`             | `event.preventDefault()`           |
| `dom.event.do/stop-propagation`            | `event.stopPropagation()`          |
| `dom.event.do/stop-immediate-propagation`  | `event.stopImmediatePropagation()` |

The attribute's presence in the concept's `with:` map is the
signal — no value, no `as:` (there's nothing to read). The
runtime sees a `the:` under `dom.event.do` and calls the method
before the handler returns.

```yaml tonk=eval
command!: &save
  with:
    body:
      description: The note body to save
      the: dom.event.current-target.elements.body/value
      as:  text
    prevent-default:
      description: Prevent the form's native submission
      the: dom.event.do/prevent-default
```

```yaml tonk=parse
view!:
  this: note
  show:
    ui: |
      <form onsubmit=save>
        <textarea name="body"></textarea>
        <button type="submit">save</button>
      </form>
```

The submit fires on the `<form>`, so `current-target` is the
form and `elements.body` is the named `<textarea>` — its
`value` becomes the `body` field. The lookup is by the control's
`name` (here `name="body"`), but remember each path segment is
camelCased first: a multi-word field must be named in camelCase
(`name="noteBody"`, read as `…elements.noteBody/value`) — a
kebab `name="note-body"` won't resolve, since the path looks up
`noteBody`. `preventDefault` fires synchronously so the page
doesn't reload.

**The prevent-default trap: an action field makes a command
rule-proof.** A `dom.event.do/*` field projects as a side effect and
stores **no value**, but a rule's `assert: <command>` premise
requires every declared field present — so the premise matches zero
rows and the rule never fires, even though the command transacts
successfully. A form-submit command like `save` above is only for
**handler-consumed** commands (typed Rust handlers). A command a
`rule!:` consumes must NOT declare a `dom.event.do/*` field: fire it
from a `<button type="button" onclick=…>` instead (no native submit
exists, so nothing needs preventing) and read the form's controls
via `dom.event.current-target.form.elements.<name>/value`.

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
    todo:
      description: The todo to complete
      the: dom.event.current-target.dataset/todo
      as:  entity

rule!:
  retract!: todo
  when:
    - assert: complete
      where: { todo: ?this }
```

The rule fires on the click-derived `complete` transient and
retracts the matched `todo`. The view's subscription sees the
row disappear.

### Minting a new entity from a rule (create forms)

A rule head needs every field bound, **including `this`** — there is
no formula that generates a fresh entity. To create durable state
from an event (a "new post" form), bind the transient command's own
content-derived entity as the new fact's identity. Fire the command
from a plain `type="button"` (NOT a form submit — see the
prevent-default trap below) and reach the form controls through
`current-target.form`:

```yaml tonk=parse
command!: &publish
  with:
    body: { description: The post body to publish, the: dom.event.current-target.form.elements.body/value, as: text }

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
      <form>
        <textarea name="body"></textarea>
        <button type="button" onclick=publish>Post</button>
      </form>
```

`type="button"` means there is no native submit to prevent, so the
command needs no `dom.event.do/prevent-default` field — which is what
keeps it rule-consumable.

One durable entity per event, carrying the command's identity. The
caveat: a command's entity is content-derived, so two events with
identical parameters collide on the same entity. If duplicates must
stay distinct, add a per-event nonce field to the command (e.g.
`time: { the: dom.event/time-stamp, as: float }`) so each event's
body — and therefore its entity — differs.

Everything the head asserts must be bound in the body: read what the
user typed from the form, and pull the rest (author, defaults) from
persistent facts joined in additional `when:` premises, or bind
constants with `==` / formula premises.

### Debugging: the click did nothing

The failure modes, from loud to silent:

1. **Console warn "field … did not resolve against the event"** —
   the path itself is wrong: a missing property step, a kebab-case
   control name (each path segment camelCases, so `name="like-seed"`
   is looked up as `likeSeed` — prefer single-word lowercase
   `name=`s), or a value that won't coerce to `as:` (an `entity`
   needs a `:`). The whole binding aborts; nothing posts.
2. **No warn, but the rule didn't fire** — a form control resolved
   blank (`""`/`null`), so the field was omitted and the posted
   command doesn't satisfy the rule's premise. See the blank-leaf
   bullet above.
3. **The command transacts (POST 200) but the rule never fires** —
   the command declares a `dom.event.do/*` field. Action fields
   store no value, so a rule premise over that command matches zero
   rows, always. See "the prevent-default trap"; fire the command
   from a `type="button"` click instead.
4. **The wrong rule fired (or two did)** — commands match
   structurally, not by name; see "One command, one shape".
5. **The form reloaded the page** — the command failed to build
   (case 1), so its queued `prevent-default` never ran and the
   native submit won.

To isolate a rule from the DOM wiring, assert the command's shape
directly with `tonk eval` and check the rule's output fact appears.
Caveat: notation forces a value into every field — including ones the
real DOM event can't produce (a blank input, an action field) — so a
rule that fires under eval can still be unreachable from the DOM;
cases 2 and 3 are exactly that false positive.

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
dispatches `CustomEvent`s consumed by commands via
`dom.event.detail/*`, exactly like the built-in
`<tonk-sheet-binder>`. Components are authored as branch data
(the `component` concept) and stay inside the concept-and-rule
pipeline; see `tonk help views`.

Anything that instead needs a whole isolated page — third-party
embeds, self-contained canvas apps — belongs in a sandboxed
iframe view (`<tonk-portal>`), outside the scope of this guide.

---

Don't memorize concept fields — use `tonk concept` and
`tonk show <concept>`. Rules themselves remain notation documents;
keep the source that installed them under version control.
