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
   transient concept — `command!:` is the keyword `tonk schema`
   shows and is equivalent to `concept!:` with `transient: true`;
   both write a fact that lives one cycle and is never persisted.
3. A **rule** (`rule!:`) watches for the transient and asserts
   downstream state. The transient is the trigger; its one-cycle
   lifetime makes the rule fire exactly once per event.

No JavaScript. No `fetch`. No `globalThis.tonk`. The bridge into
the worker is the transient assertion, posted by the runtime
when the event fires; you describe the projection, not the wire
call.

### Wiring a click

```yaml
attribute!: &count
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

view!: &counter-basic
  model: counter
  display: !text/html |
    <p>{count}
      <button onclick=increment data-subject={this}>+</button>
    </p>
```

A view is identified by its **anchor name** (`&counter-basic`
publishes it under `id:counter-basic`). It declares the `model`
it renders and the `display` template; `<tonk-display>` resolves
the view by that name. Re-asserting the same anchor with a
different `display` re-points the name to the new entity — edits
replace in place, so there are never stale duplicates to pick
between.

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
- The `as:` type drives coercion: `text` → string, `entity` →
  string parsed as a URI (rejected if it has no `:`),
  `unsigned-integer` / `signed-integer` / `float` → number,
  `boolean` → the JS value must already be a boolean (e.g.
  `event.shiftKey`); a non-boolean fails to resolve rather than
  being coerced.

### Passing view context via `data-*`

A click handler doesn't implicitly know the row it was rendered
for. Surface what you need on the element with `data-*` and
read it back through `dom.event.current-target.dataset/<name>`:

```yaml
view!: &todo-row
  model: todo
  display: !text/html |
    <li>
      <span>{title}</span>
      <button onclick=complete data-todo={this}>done</button>
    </li>

command!: &complete
  with:
    todo:
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

```yaml
command!: &save
  with:
    body:
      the: dom.event.current-target.elements.body/value
      as:  text
    prevent-default:
      the: dom.event.do/prevent-default
```

```yaml
view!: &save-form
  model: note
  display: !text/html |
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

### `rule!:` — the reactive layer

A rule has a head with one polarity and a body of premises:

```yaml
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

```yaml
command!: &complete
  with:
    todo:
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

### Opening the view in tonk-ui

Once the view and the entity it renders are asserted, push the
repo and mint a launcher URL with `tonk share display`:

```
tonk share display <subject> --view <view-name>
```

- `<subject>` is the bookmark name or `did:key:…` URI of the
  entity to render — the value `<tonk-display>` reads as its
  `entity` attribute.
- `--view <view-name>` is the view's **anchor name** (the `&name`
  on its `view!:`). `<tonk-display>` resolves it to the view
  entity the name currently points at, then reads that view's
  `model` (to project the subject's fields) and `display` (the
  template). Omit `--view` and the element falls back to
  carousel mode — every view published for the subject's model.

With `--view`, you don't pass `--model`: the named view declares its
own `model`, so the caller never repeats it. (`--model` is the carousel
form — see `tonk guide views`.) The `view` concept is `{model,
display}`; identity lives in the anchor name, not a `name` field.

Concrete example, given the counter setup above:

```
tonk share display my-counter --view counter-basic
```

The launcher URL points at the recipient's
`/space/<space-name>/branch/main/display/my-counter?view=counter-basic`
route. `<tonk-display>` resolves `id:counter-basic` to the
current view entity, opens its subscriptions, and the
`on<event>=<concept>` bindings in the template fire rules
end-to-end. Edit the view by re-asserting `view!: &counter-basic`
with new `display` — the name re-points and a refresh shows the
latest, with no duplicate rows accumulating.

Two related verbs to keep apart:

- `tonk share view` targets the iframe viewer
  (`/branch/main/view/<entity>`), driven off entities carrying a
  `text/html` claim. Useful for one-off HTML pages, but the
  shell that route serves doesn't currently register
  `<tonk-display>`, so events won't fire.
- `tonk share concept` targets the auto-rendered concept
  listing — no template authoring, no events.

`tonk share display` is the verb that pairs with this guide.

### When the declarative path doesn't fit

Anything that needs to render arbitrary HTML/CSS/JS without
going through the concept-and-rule pipeline — third-party
embeds, complex canvas drawing, a charting library — belongs
in a sandboxed iframe view rather than in the main view body.
That sandbox is a separate piece (`<tonk-portal>`) outside the
scope of this guide.

---

Don't memorize built-ins — run `tonk schema` to see the concepts,
rules, and transient commands already on the branch.
