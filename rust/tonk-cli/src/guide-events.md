# Reactive views: nominal commands and event projections

Interactive markup names a stable command or projection with an `on<event>`
attribute. The browser resolves stored branch data, extracts typed arguments,
runs declared synchronous event actions, and posts an `op: invoke` claim. Rules
and native handlers are selected by the command kind before arguments are
decoded. Command arguments never become ordinary transient facts.

The three declarations have separate jobs:

1. `command!:` defines a stable kind and typed semantic arguments.
2. `projection!:` maps a particular event shape to those arguments and lists
   synchronous actions such as `prevent-default`.
3. `rule!:` consumes the nominal kind transactionally and writes durable data.

## Copy-runnable list append

```yaml
attribute!: &todo-title
  description: Todo text
  the: xyz.tonk.todo/title
  as: text
  cardinality: one

attribute!: &todo-list-ref
  description: List containing the todo
  the: xyz.tonk.todo/list
  as: entity
  cardinality: one

attribute!: &todo-list-name
  description: List label
  the: xyz.tonk.todo-list/name
  as: text
  cardinality: one

concept!: &todo/item
  description: One persistent todo membership
  with:
    title: todo-title
    list: todo-list-ref

concept!: &todo/list
  description: One persistent todo list
  with:
    name: todo-list-name

todo/list!: &todo-list
  name: Todos

command!: &todo/add
  description: Append one todo to a list
  with:
    title:
      description: Todo text
      the: xyz.tonk.command.todo.add/title
      as: text
    list:
      description: Destination list
      the: xyz.tonk.command.todo.add/list
      as: entity

projection!: &todo/add-form
  command: todo/add
  default: true
  arguments:
    title: { control: "title" }
    list: { literal: id:todo-list }
  actions:
    - prevent-default

rule!:
  this: id:todo/add-rule
  description: Persist each projected todo
  assert!: todo/item
  when:
    - assert: todo/add
      where: { this: ?this, title: ?title, list: ?list }

view/directory!: &todo-list-view
  model: todo/list
  display: !text/html |
    <section class="todo-list">
      <form onsubmit=todo/add>
        <input name="title" aria-label="New todo">
        <button type="submit">Add</button>
      </form>
      <ul><tonk-display model=todo/item /></ul>
    </section>

view/directory!: &todo-item-view
  model: todo/item
  display: !text/html |
    <li data-todo={this}>{title}</li>
```

Evaluate it, then verify the projection without opening a browser:

```yaml
# fixture.yaml
controls:
  title:
    value: Buy milk
```

```sh
tonk eval todo.notation
tonk project todo/add-form --fixture fixture.yaml --json
tonk home todo/list                 # a build nobody can see isn't done
tonk render todo/list               # headless check that it renders
```

`project` is read-only. Add `--transact` only when you intend to run the
declarative rule. `--redact` replaces values while retaining field and source
names for shareable diagnostics.

`home` is not optional bookkeeping: without it the views above resolve
correctly and stay invisible, because nothing points the space home at them.

## Projection sources

Each command argument has exactly one source:

```yaml
arguments:
  title:   { control: "note-body" }
  done:    { control: { name: "done", property: checked } }
  subject: { data: "subject" }
  key:     { event: key }
  href:    { detail: "href" }
  value:   { target: value }
  list:    { literal: id:todo-list }
```

Control and `data-*` names are exact: `note-body` stays `note-body`; it is
never camel-cased. Supported event members are `type`, `key`, `code`,
`repeat`, `shiftKey`, `ctrlKey`, `altKey`, `metaKey`, `button`, `clientX`,
`clientY`, and `timeStamp`. Target members are `value` and `checked`.

A present empty text value is `""`, not absence. A missing optional source
omits that argument. A missing required source, failed read, or failed type
coercion aborts the whole projection and executes no actions.

## Event actions

Actions run synchronously, in declaration order, after every source has been
read and validated:

```yaml
actions:
  - prevent-default
  - stop-propagation
  - stop-immediate-propagation
```

Actions are projection behavior, never command arguments and never wire
payload fields.

## Binding resolution

An event binding may name:

- a projection, which is used directly;
- a command with one projection;
- a command with several projections and exactly one `default: true`.

Several projections without one unique default are an error. A compatibility
structural command is considered only when the reference resolves to no
nominal command or projection. One event invocation is consumed by exactly one
lane.

## Diagnostics

Common failures are explicit:

- `missing required field "title"`: the named control/source was absent;
- `required command argument "title" is missing`: an invocation omitted a
  required schema argument;
- `command has multiple projections and no unique default`: bind the explicit
  projection or mark one default;
- `command_unhandled`: no declarative rule or native handler is registered;
- handled with `registered_rules > 0` and `fired_rules = 0`: the command was
  valid, but its durable premises did not match;
- native status `failed`: query `/api/invocations/<correlation>` for the
  sanitized handler failure. The triggering revision remains committed.

Browser diagnostics contain the projection, command, field, and source, but
omit argument values. Use `tonk project ... --redact` when sharing a headless
trace.

## Compatibility

Legacy structural `dom.event/*` descriptors remain supported only for branches
that have not migrated. New code should use nominal declarations and stored
projections. `tonk commands inventory --json` distinguishes both forms and
pins the observed branch revision for migration review.
