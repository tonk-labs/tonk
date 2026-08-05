#!/usr/bin/env bash
# Known-good nominal command/projection reference path.
set -euo pipefail

TONK="${TONK:?}"

"$TONK" eval -c '
attribute!: &todo-title
  description: Todo text
  the: bench.todo/title
  as: text
  cardinality: one

attribute!: &todo-list-ref
  description: List containing the todo
  the: bench.todo/list
  as: entity
  cardinality: one

attribute!: &todo-list-name
  description: List label
  the: bench.todo-list/name
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
    title: { description: Todo text, the: bench.command.todo.add/title, as: text }
    list: { description: Destination list, the: bench.command.todo.add/list, as: entity }

projection!: &todo/add-form
  command: todo/add
  default: true
  arguments:
    title: { control: "title" }
    list: { literal: id:todo-list }
  actions: [prevent-default]

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
'

"$TONK" home todo/list
