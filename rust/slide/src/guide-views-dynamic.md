## Reactive views: events and effects

Templates with `{field}` interpolation cover the read path: a
view subscribes to a query and re-renders rows in place when the
data changes. Interactivity — clicks, form submits, anything a
user does — flows the other direction, and it is also fully
declarative.

The model has three pieces:

1. **`on<event>=<concept>`** on a template element names the
   concept a DOM event produces.
2. A **transient concept** describes the shape of that event-
   derived fact and where each field reads from.
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

# The event-derived concept. `transient: true` is what tells the
# reactor not to persist it.
concept!: &increment
  transient: true
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

Rules of the translation:

- Split the identifier on `.` and `/`. Each segment is a
  successive property step on the event object.
- Within a segment, kebab-case becomes camelCase before lookup
  (`shift-key` → `shiftKey`, `item-id` → `itemId`).
- A leading `target` segment is `event.target` — the *deepest*
  node the event fired on. A leading `current-target` segment
  is the **bound element**: the closest ancestor of
  `event.target` that carries the `on<event>` you authored.
  (This is not the literal `event.currentTarget`, which points
  at the host where the delegated listener lives.) To read a
  `data-*` you put on the element with the handler, always use
  `current-target` — `target` may be a child you didn't author
  (e.g. a `<span>` inside the button).
- The path is evaluated live, at handler-fire time. A missing
  path (e.g. `event.pressure` on a `MouseEvent`) omits that
  field from the asserted fact.
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

concept!: &complete
  transient: true
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
concept!: &save
  transient: true
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
`value` becomes the `body` field. `preventDefault` fires
synchronously so the page doesn't reload.

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
concept!: &complete
  transient: true
  with:
    todo:
      the: dom.event.target.dataset/todo
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
repo and mint a launcher URL with `slide share display`:

```
slide share display <subject> --view <view-name>
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

There is no `--model`: the view declares its own `model`, so the
caller never repeats it. The `view` concept is `{model,
display}`; identity lives in the anchor name, not a `name` field.

Concrete example, given the counter setup above:

```
slide share display my-counter --view counter-basic
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

- `slide share view` targets the iframe viewer
  (`/branch/main/view/<entity>`), driven off entities carrying a
  `text/html` claim. Useful for one-off HTML pages, but the
  shell that route serves doesn't currently register
  `<tonk-display>`, so events won't fire.
- `slide share concept` targets the auto-rendered concept
  listing — no template authoring, no events.

`slide share display` is the verb that pairs with this guide.

### What this replaces

Earlier drafts of this guide pointed at a `globalThis.tonk` API
(`tonk.query`, `tonk.subscribe`, `tonk.evaluate`) that lived on
the window inside the served view iframe. That API still backs
the host elements internally, but it is no longer the agent
surface: writing views as `<script type="module">` blocks that
call into it is a stopgap, not a pattern to lean on.

The declarative path is the right one because every piece is
the same machinery the rest of the system already uses:
concepts, rules, queries, subscriptions. Adding a new
interaction means adding a transient concept and a rule, not
inventing a new request shape on the wire.

### When the declarative path doesn't fit

Anything that needs to render arbitrary HTML/CSS/JS without
going through the concept-and-rule pipeline — third-party
embeds, complex canvas drawing, a charting library — belongs
in a sandboxed iframe view rather than in the main view body.
That sandbox is a separate piece (`<tonk-portal>`) outside the
scope of this guide.
