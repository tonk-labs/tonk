# Reactive views: events and effects

A view renders data; an interaction changes it. Both are declarative:
a template attribute `on:<name>=<command>` posts a **command** (a
transient fact that lives for one reactor cycle and is never stored)
when the user acts, and a **rule** turns that command into durable
state. No JavaScript, no `fetch`.

## The pattern

```yaml tonk=eval
attribute!: &count
  description: The counter's current value
  the: xyz.tonk.counter/count
  as:  natural

concept!: &counter
  with:
    count: count

# The command: what the click means, in domain terms.
command!: &increment
  with:
    subject:
      description: The counter entity to increment
      the: xyz.tonk.counter.increment/subject
      as:  entity

# The rule: when an increment arrives, write the new count.
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

A click posts `increment` with `subject` set to the counter the button
was rendered for. The rule reads it with the current `counter`, asserts
the new count, and the view re-renders. The `increment` fact is gone
next cycle.

- `on:click` names an **event declaration** (`on/click`, built in) that
  says which platform event to listen for and how to fill the command's
  fields. The value (`increment`) is the command, by name or by entity
  URI (`on:join=tonk:join`).
- `command!:` is `concept!:` with `transient: true`.
- A rule needs at least one positive `when:` premise over a command;
  that premise is its trigger.

## Built-in events

| Attribute       | Listens for | Fills                                                      | Also                |
|-----------------|-------------|------------------------------------------------------------|---------------------|
| `on:click`      | `click`     | `subject` ← `{this}`                                       |                     |
| `on:click-once` | `click`     | `subject` ← `{this}`, `time` ← `.timeStamp`                |                     |
| `on:submit`     | `submit`    | `subject` ← `{this}`                                       | `prevent-default`   |
| `on:change`     | `change`    | `subject` ← `{this}`, `value` ← `.currentTarget.value`     |                     |
| `on:toggle`     | `change`    | `subject` ← `{this}`, `checked` ← `.currentTarget.checked` |                     |

`{this}` is the entity the element was rendered for: the row's subject
in a directory, the displayed entity in a single-entity view.

**The command's fields must be exactly the fields the declaration
fills.** For `on:toggle` that is `subject` (`as: entity`) and `checked`
(`as: boolean`); for `on:click-once`, `time` is a `float`. Every
required (`with:`) field needs a source, and every source needs a field
on the command. When the declaration is in your document, `tonk eval`
checks this (`E_EVENT_COMMAND_MISMATCH`). **The built-ins live on the
branch, so nothing checks a command against them**: a command whose
field is called `counter` instead of `subject` installs cleanly, posts
without the field, and the rule never fires. If a command needs any
other shape, declare your own event in the same document (see "The
`event` concept" below).

For a checkbox, render stored state with `checked={done}` and bind the
change: `<input type="checkbox" checked={done} on:toggle=toggle-task>`.
A bare `{done}` inside the tag does not set the checked property.

## Recipes

### Remove a row

```yaml tonk=eval
attribute!: &todo/title
  description: What to do
  the: xyz.tonk.todo/title
  as: text

concept!: &todo
  with:
    title: todo/title

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
    - assert: todo
      where: { this: ?this, title: ?title }

view!:
  this: todo
  show:
    ui: |
      <li>{title} <button on:click=complete>done</button></li>
```

### Pass something other than the row

Render it as a `data-*` attribute and read it in your own declaration:

```yaml tonk=eval
attribute!: &todo/title
  description: What to do
  the: xyz.tonk.todo/title
  as: text

attribute!: &todo/list
  description: The list the todo belongs to
  the: xyz.tonk.todo/list
  as: entity

concept!: &todo
  with:
    title: todo/title
    list: todo/list

event!: &on/todo-move
  type: "click"
  where:
    subject: "{this}"
    list: ".currentTarget.dataset.list"

command!: &todo/move
  with:
    subject: { description: The todo to move, the: xyz.tonk.todo.move/subject, as: entity }
    list: { description: The list to move it to, the: xyz.tonk.todo.move/list, as: entity }

rule!:
  assert!: todo
  when:
    - assert: todo/move
      where: { subject: ?this, list: ?list }
    - assert: todo
      where: { this: ?this, title: ?title }

view!:
  this: todo
  show:
    ui: |
      <li>
        <span>{title}</span>
        <button on:todo-move=todo/move data-list={list}>move</button>
      </li>
```

`data-list={list}` on the bound button is read back as
`.currentTarget.dataset.list` (or as `"{list}"`).

### Submit a form

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

`.currentTarget` is the bound `<form>`; `elements.body` is its control
named `body`. `prevent-default` stops the page reload.

### Create a new entity

A rule head needs every field bound, including `this`, and no formula
mints a fresh entity. Use the command's own entity: it is derived from
the command's content, so add a `time` (`.timeStamp`) to make each event
distinct.

```yaml tonk=eval
attribute!: &post/body
  description: A post's text
  the: xyz.tonk.post/body
  as: text

concept!: &post
  with:
    body: post/body

concept!: &board
  with:
    title: { description: The board's title, the: xyz.tonk.board/title, as: text }

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
      where: { this: ?this, body: ?body }

view!:
  this: board
  show:
    ui: |
      <form on:post-publish=publish>
        <textarea name="body"></textarea>
        <button type="submit">Post</button>
      </form>
```

Everything else the head needs (author, defaults) comes from more
`when:` premises over stored facts.

## Events from custom elements

An element you define with `element!:` (see `tonk help views`, "Web
components") reports what happened by dispatching a `CustomEvent`.
Bind it like a click: declare an event whose `type` is the custom
event's name and whose sources read `.detail.*`.

```yaml tonk=eval
element!: &tally-widget
  description: "A running tally; a click on it dispatches a bump event"
  method:
    connected: |
      (self) => {
        self.textContent = `count: ${self.getAttribute('count') ?? 0}`;
        self.__click ??= () => self.bump();
        self.addEventListener('click', self.__click);
      }
    attribute-changed: |
      (self, name, before, after) => {
        if (name === 'count') self.textContent = `count: ${after ?? 0}`;
      }
    bump: |
      (self) => self.dispatchEvent(
        new CustomEvent('bump', { bubbles: true, detail: { amount: 1 } }))

attribute!: &tally/count
  description: The tally's current value
  the: xyz.tonk.tally/count
  as: natural

concept!: &tally
  with:
    count: tally/count

event!: &on/tally-bump
  type: "bump"
  where:
    subject: "{this}"
    amount: ".detail.amount"

command!: &tally/bump
  with:
    subject: { description: The tally to bump, the: xyz.tonk.tally.bump/subject, as: entity }
    amount: { description: How much to add, the: xyz.tonk.tally.bump/amount, as: natural }

rule!:
  assert!: tally
  when:
    - assert: tally/bump
      where: { subject: ?this, amount: ?amount }
    - assert: tally
      where: { this: ?this, count: ?old }
    - assert: math/sum
      where: { of: ?old, with: ?amount, is: ?count }

view!:
  this: tally
  show:
    ui: |
      <tally-widget count={count} on:tally-bump=tally/bump></tally-widget>
```

Requirements, all from how the runtime listens:

- The view's `<tonk-display>` installs one listener for each bound
  declaration's `type`, any name, on itself. The event must reach it, so
  dispatch with **`bubbles: true`**. Dispatching on the element itself
  (`self.dispatchEvent`) needs nothing else; an event dispatched from
  inside the element's own shadow root also needs `composed: true`.
- The `on:` attribute goes on the element that dispatches (or an
  ancestor of it inside the same view). The runtime walks up from the
  event's target to the nearest element with a matching binding.
- The listener is installed after the view renders. An event fired from
  `connected` can arrive before it: wait for the enclosing
  `tonk-display` to carry `data-bound`, the way the library's
  `<page-mount>` does, or fire only in response to the user.
- `.detail.<key>` is spelled exactly as the element spells it
  (`detail.sheetName` is `.detail.sheetName`), and its value must
  already have the JS type the command field's `as:` wants: a number for
  `natural`/`integer`/`float`, a boolean for `boolean`, a string for
  `text`, a string containing `:` for `entity`.
- The declaration's name is yours to choose; it need not match the
  event type (`on/tally-bump` listens for `bump`). Prefix it with the
  element's name to keep declarations from colliding.

The standard library's `<tonk-table>` and `<tonk-notebook>` views are
bound exactly this way.

## The `event` concept

An `event!:` declaration is schema, like `command!:` and `rule!:`: a
durable fact on the branch saying how one platform event fills a
command's fields. It is the only place DOM knowledge lives; the command
stays domain-shaped, so a different host can declare its own event for
the same command.

### Naming

A template attribute `on:<name>` resolves the declaration named
`on/<name>` (`/` is not allowed in an HTML attribute name, `:` is). So
publish the declaration with the anchor `&on/<name>`. A declaration
under any other name is unreachable from a template. Only the `on:`
prefix is reserved; `xlink:href` and the like are untouched.

### Fields

| Field              | Required | Meaning                                                            |
|--------------------|----------|--------------------------------------------------------------------|
| `type`             | yes      | The platform event name to listen for (`click`, `submit`, `bump`). |
| `where`            | yes      | Map from **command field name** to source.                        |
| `prevent-default`  | no       | Call `event.preventDefault()` when this binding posts.             |
| `stop-propagation` | no       | Call `event.stopPropagation()` when this binding posts.            |

There are no other fields. The flags run synchronously in the handler,
only for the binding that posts, and only once every source resolved.

The built-ins are ordinary declarations in the standard library:

```yaml tonk=eval
event!: &on/click
  type: "click"
  where:
    subject: "{this}"

event!: &on/click-once
  type: "click"
  where:
    subject: "{this}"
    time: ".timeStamp"

event!: &on/submit
  type: "submit"
  prevent-default: true
  where:
    subject: "{this}"

event!: &on/change
  type: "change"
  where:
    subject: "{this}"
    value: ".currentTarget.value"

event!: &on/toggle
  type: "change"
  where:
    subject: "{this}"
    checked: ".currentTarget.checked"
```

Two declarations can listen for the same `type` and read it
differently (`on/change` and `on/toggle`).

### Sources

Quote the `{…}` and `.…` forms: a bare `{this}` is a YAML mapping.

| Source                                  | Reads                                                                |
|-----------------------------------------|----------------------------------------------------------------------|
| `"{this}"`                              | the row's subject, or the `<tonk-display>`'s entity                  |
| `"{name}"`                              | the `data-name` attribute on the bound element or nearest ancestor   |
| `".key"`, `".timeStamp"`, `".shiftKey"` | a property of the live event                                         |
| `".detail.amount"`                      | a `CustomEvent`'s `event.detail.amount`                              |
| `".currentTarget.value"`                | the bound element's `value`                                          |
| `".currentTarget.dataset.list"`         | the bound element's `data-list`                                      |
| `".currentTarget.elements.body.value"`  | a bound form's control named `body`                                  |
| `"draft"`                               | the constant text `draft`                                            |
| `inbox`, `did:key:…`                    | the entity the name refers to, or that entity URI                    |

- A `.path` is read the way JavaScript spells it, segment by segment,
  with no case conversion (`.currentTarget.elements.note-body.value`).
- `.currentTarget` is the **bound element**, the one carrying the `on:`
  attribute, not the listener's host. `.target` is the deepest node hit
  (perhaps a `<span>` inside the button), so prefer `.currentTarget`.
- `"{name}"` needs the template to render `data-name={name}`. When the
  declaration is in the document, `tonk eval` rejects a `{name}` the
  view's concept does not declare (`E_UNKNOWN_EVENT_SOURCE_FIELD`).
- A bare name that names nothing fails at `tonk eval`
  (`E_UNKNOWN_NAME_REFERENCE`).

The command field's `as:` decides how the value is taken:

| `as:`                          | accepts                                          |
|--------------------------------|--------------------------------------------------|
| `text`                         | a string                                         |
| `entity`                       | a string containing `:` (a URI)                  |
| `natural`, `integer`, `float`  | a JS number (`natural`/`integer`: no fraction)   |
| `boolean`                      | a JS boolean                                     |

An input's `.value` is always a string, so read it into a `text` field
and convert in the rule. Constants are text, so they fill only `text`
fields and (when they contain `:`) `entity` fields.

At event time:

- A source that does not resolve (a missing property, a `data-*` the
  template did not render, a value of the wrong type) means the binding
  does not apply: nothing posts, nothing is logged, and the event falls
  through to the next bound ancestor.
- A **blank** value (`""` from an empty input, `null`) is omitted and
  the command posts without it. A rule premise naming that field then
  matches nothing. Put a field that may be empty under the command's
  `maybe:` and leave it out of the rule premise.
- When nested elements bind the same event, the innermost binding that
  resolves wins.

### One declaration, many commands

A declaration fills any command whose fields are exactly its `where:`
keys (with `maybe:` fields optional), so reuse one whenever the shape
matches; `on/click` serves every command that takes just `subject`.
When a command needs a different set of fields, write a new declaration
rather than renaming fields to fit.

Commands are matched **structurally**: a rule premise `assert:
<command>` matches any transient carrying all of that command's
attributes. Two commands built from the same attributes are the same
shape, and one event fires both rules. `tonk eval` rejects such pairs
(`E_OVERLAPPING_TRANSIENT_COMMANDS`), so give each command its own
attribute namespace even when field names match:

```yaml tonk=eval
command!: &activate-page
  with:
    page: { description: The page to activate, the: xyz.tonk.page.activate/page, as: entity }
command!: &delete-page
  with:
    page: { description: The page to delete, the: xyz.tonk.page.delete/page, as: entity }
```

### What `tonk eval` checks

When it installs a view, for every `on:` binding:

| Code                            | When                                                             |
|---------------------------------|------------------------------------------------------------------|
| `E_UNKNOWN_EVENT_DECLARATION`   | no `on/<name>` exists, in the document or on the branch          |
| `E_UNKNOWN_BOUND_COMMAND`       | the command name or URI resolves to nothing                      |
| `E_EVENT_COMMAND_MISMATCH`      | a required field has no source, or a source fills no field       |
| `E_UNKNOWN_EVENT_SOURCE_FIELD`  | a `{name}` source the view's concept does not declare            |

The last two run only when the declaration is in the same document.
A declaration already on the branch, including every built-in, is
resolved by name at render time and is not checked against the command.

## Rules

```yaml tonk=illustrative-placeholders
rule!:
  assert!: <head-concept>      # or `retract!: <head-concept>`
  when:
    - assert: <concept>
      where: { <field>: ?var, ... }
  unless:                       # optional negative premises
    - assert: <concept>
      where: { ... }
```

- Head fields come from same-named variables in the body: a `counter`
  head (fields `this`, `count`) takes `?this` and `?count`.
- At least one positive `when:` premise must read a command; a rule
  with only stored premises is rejected (use a query instead).
- Several rules can share a head; a new interaction is a new rule.
- Formulas such as `math/sum` (`{of: ?a, with: ?b, is: ?c}`) and
  comparisons work as premises.

## Debugging: the click did nothing

1. **`tonk eval` rejected the view**: fix what the error code above
   names.
2. **The command's fields don't match a built-in declaration**: nothing
   checks this; compare against the built-in table, or declare your own
   event in the document so `tonk eval` does.
3. **A source didn't resolve**: misspelled path, `data-*` not rendered,
   or a type that won't coerce. Nothing posts. For a form, the native
   submit then reloads the page, since `prevent-default` never ran.
4. **The command posted but the rule didn't fire**: a blank field was
   omitted (see `maybe:` above).
5. **The wrong rule fired, or two did**: commands match structurally.
6. **A custom element's event is ignored**: it did not bubble, was
   fired before the view was bound, or its `detail` value has the wrong
   JS type.

To test a rule without the DOM, assert the command directly with `tonk
eval` and check the rule's output. That proves the rule, not the
wiring: eval sets fields the real event may never fill (cases 2 to 4).

Events fire only in the live shell: open `/space/<space>/<model>`, or
`/space/<space>/<entity>@<model>!<view>` for one entity. `tonk render`
prints markup with nothing behind it. A visual click proves neither
persistence nor sync: read the record back, reload, and check a second
client.

For a whole isolated page (third-party embeds, canvas apps) use a
`portal` view instead (`tonk help views`).
