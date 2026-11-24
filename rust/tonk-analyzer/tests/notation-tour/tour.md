# A tour of the Dialog notation

This notebook is a tour of the notation Dialog is growing: s-expressions over a small kernel, where the core forms (`concept`, `select`, `query`, commands) are themselves defined as rules. Every notation code block below is a cell. Press Run (or Cmd/Ctrl+Enter in the cell) and the space's worker reads it, expands it, analyzes it and compiles it with `dialog-notation`, then runs what it compiled against this space's branch.

Under each result, four sections show the stages: read (the syntax trees, and each tree as data), expanded (every form rewritten to a rule of the kernel), analysis (what the cell declares, and every diagnostic with its place) and compiled (the steps Dialog-DB runs, its descriptors as JSON). The cells above a cell are its context: their definitions are in scope, the way a later paragraph reads an earlier one. Run the cells from the top: a cell reads the facts the cells above it wrote.

## Facts and concepts

A concept names a shape of facts. This one is a to-do: a title, a state that is one of two, and the list it is on. Each line is one attribute, with its type and its cardinality. Every line of a concept says its cardinality: `:one` keeps the newest claim, `:many` keeps them all.

```notation
; A thing to do.
(concept (todo ?t title: ?title state: ?state list: ?list)
  (tour.todo/title of: ?t is: ?title as: :text cardinality: :one)
  (tour.todo/state of: ?t is: ?state as: [:open :done] cardinality: :one)
  (tour.todo/list of: ?t is: ?list as: :entity cardinality: :one))

; A list of things to do.
(concept (list ?l name: ?name)
  (tour.list/name of: ?l is: ?name as: :text cardinality: :one))
```

Asserting through a concept writes one fact per line. A write through a concept says every field, or the entity alone; one field is written through the attribute's relation.

```notation
(assert (list list:home name: "Home"))
(assert (list list:work name: "Work"))
(assert (list list:garden name: "Garden"))
(assert (todo todo:milk title: "Buy milk" state: :open list: list:home))
(assert (todo todo:mum title: "Call mum" state: :done list: list:home))
(assert (todo todo:deck title: "Write the deck" state: :open list: list:work))
(assert (todo todo:seeds title: "Sow the seeds" state: :done list: list:garden))
```

And asking reads them back. A variable binds. A read names every field, or ends with `...` to say it leaves the others out on purpose.

```notation
(select (todo ?t title: ?title state: ?state list: ?list))

(select (todo ?t title: ?title ...))
```

## Queries

A query of several premises joins them on their shared variables. Its head is a shape of its own: here, what is open and the name of its list.

```notation
(select (tour.open ?t title: ?title list: ?name)
  (todo ?t title: ?title state: :open list: ?l)
  (list ?l name: ?name))
```

Negation: lists with nothing open on them. Every variable of an `unless` is bound by a premise before it.

```notation
(select (tour.clear ?l name: ?name)
  (list ?l name: ?name)
  (unless (todo _ state: :open list: ?l ...)))
```

Comparisons and arithmetic are formulas. Give each to-do an estimate, then ask for the quick ones, and for pairs that fit in three hours, with what each pair adds up to.

```notation
(concept (estimate ?t hours: ?h)
  (tour.todo/hours of: ?t is: ?h as: :natural cardinality: :one))

(assert (estimate todo:milk hours: 1))
(assert (estimate todo:mum hours: 1))
(assert (estimate todo:deck hours: 3))
(assert (estimate todo:seeds hours: 2))

(select (tour.quick ?t title: ?title hours: ?h)
  (todo ?t title: ?title ...)
  (estimate ?t hours: ?h)
  (< ?h : 2))

(select (tour.morning ?a with: ?b total: (+ ?p : ?q))
  (estimate ?a hours: ?p)
  (estimate ?b hours: ?q)
  (< ?p : ?q)
  (<= (+ ?p : ?q) : 3))
```

A fold: a `query` collects with `collect` and counts. Here, one summary per list with how many things are on it.

```notation
(query (list/summary ?l is: ?summary)
  (list ?l name: ?name)
  (todo (collect ?t) list: ?l ...)
  (is ?summary {:name ?name :todos (count ?t)}))

(select (list/summary ?l is: ?summary))
```

## Rules

A deductive rule derives a relation from others, and is kept on the branch: any later query reads it like stored facts. A list is busy while something on it is open.

```notation
; A list is busy while something on it is open.
(rule (tour.busy ?l name: ?name)
  (list ?l name: ?name)
  (todo _ state: :open list: ?l ...))

(select (tour.busy ?l name: ?name))
```

Rules can recurse. One to-do blocks another; blocking is transitive.

```notation
(assert (tour.blocks todo:seeds then: todo:deck))
(assert (tour.blocks todo:deck then: todo:milk))

(rule (tour.before ?a then: ?b) (tour.blocks ?a then: ?b))
(rule (tour.before ?a then: ?c) (tour.blocks ?a then: ?b) (tour.before ?b then: ?c))

(select (tour.before ?a then: todo:milk))
```

An inductive rule writes when a commit lands. A command is a concept on the control layer: asserted, it holds for one tick, the rules that read it fire, and it is not stored. Its lines say their cardinality too. Completing a to-do replaces its record: one rule retracts the record as it is, every field named, and one asserts it changed, with its state done. Both read the same premises and fire in the same round; the fields both name hold afterwards. One field alone is retracted through the attribute's relation, `(retract (tour.todo/state of: ?t is: :open))`.

```notation
(concept (todo/complete todo: ?t)
  (tour.todo.complete/todo of: ?c is: ?t as: :entity in: :control cardinality: :one))

; Completing a to-do takes back the record as it was.
(rule (retract (todo ?t title: ?title state: :open list: ?l))
  (todo/complete todo: ?t)
  (todo ?t title: ?title state: :open list: ?l))

; And asserts it done.
(rule (assert (todo ?t title: ?title state: :done list: ?l))
  (todo/complete todo: ?t)
  (todo ?t title: ?title state: :open list: ?l))
```

Now issue the command, and read the state back in the same cell: the rules fired in the commit that carried it.

```notation
(assert (todo/complete todo: todo:milk))

(select (todo todo:milk title: ?title state: ?state list: ?l))
```

## Cardinality

`cardinality: :one` keeps the last value of an attribute, `:many` keeps them all. Tags are many.

```notation
(concept (tagged ?t tag: ?tag)
  (tour.todo/tag of: ?t is: ?tag as: :text cardinality: :many))

(assert (tagged todo:deck tag: "work"))
(assert (tagged todo:deck tag: "writing"))
(assert (tagged todo:milk tag: "errand"))

(select (tagged ?t tag: ?tag))
```

A field can elect one of many claims with `max` or `min`. Over a listed type they rank by the listing, which is written least first: `max` elects the case listed last. A to-do's urgency keeps every level it was given, and reads the highest.

```notation
(concept (urgency ?t level: (max ?u))
  (tour.todo/urgency of: ?t is: ?u as: [:low :normal :high] cardinality: :many))

(assert (urgency todo:deck level: :normal))
(assert (urgency todo:deck level: :high))
(assert (urgency todo:deck level: :low))
(assert (urgency todo:seeds level: :low))

(select (urgency ?t level: ?level))
```

## Patterns over maps and lists

A value can be a record or a list, and a query takes it apart by shape. An open map matches any record that has the keys asked for; `...` names the rest.

```notation
(assert (tour.note note:1 about: {:author "Ada" :tags ["math" "engines"] :pinned true}))
(assert (tour.note note:2 about: {:author "Grace"}))

(select (tour.note ?n about: {:author ?who}))

(select (tour.note ?n about: {:author ?who :tags [?first ?second] :pinned true}))
```

The empty set is written `{:}` (and the empty map `{}`). It is stored as the empty list, so the answer reads it back as `[]`, and a pattern `{:}` finds it.

```notation
(assert (tour.note note:3 about: {:author "Lin" :tags {:}}))

(select (tour.note ?n about: {:author ?who :tags {:}}))
```

## Views

A view is a query whose answer is markup. A list's card holds an item per to-do on it, and the driver renders it to the HTML a page shows. Press Watch live under the result and the worker keeps a copy of the card current: it watches the view with the driver and streams its patches over the page's subscription, and the copy applies them. Run the cells below and the to-do the macro adds to Home lands in the watched card as an insert, with no re-run. Each item's class is its state, as the notation writes an entity as text: `open:` or `done:` (done items are struck through). Buy milk reads done here because the command cell above completed it. Each item also binds a click to the `todo/complete` command: click an open to-do in the watched card, and the command it posts comes back as a patch of its class.

```notation
(query (todo/item ?t list: ?l is: ?item)
  (todo ?t title: ?title state: ?s list: ?l)
  (view ?item [:li {:class ?s :on/click (todo/complete todo: ?t)} ?title]))

(query (list/card ?l is: ?card)
  (list ?l name: ?name)
  (todo/item _ list: ?l is: (collect ?items))
  (view ?card [:article [:h3 ?name] [:ul ?items]]))

(select (list/card list:home is: ?card))
```

## Notation as data

The notation is made of rules all the way down: `select`, `concept` and commands are macros the analyzer runs, defined in its prelude as rules over syntax. A document can define one of its own and use it in the same breath. Here `[todo! id title list]` stands for asserting an open to-do (look at the expanded stage: the bracket form is gone).

```notation
; Shorthand for an open to-do.
(rule (syntax (assert (todo ?t title: ?x state: :open list: ?l)))
  (expand [todo! ?t ?x ?l]))

[todo! todo:bread "Bake bread" list:home]

(select (todo todo:bread title: ?title state: ?state list: ?l))
```

A check is a rule over syntax too. This one says something about a form without refusing it: a to-do written as done already. The assert below still runs; its warning comes from the rule above it. (The analyzer's own "no rule reads this command" is a rule of the same kind, in its prelude.) Today such a check can only warn, never refuse, and its premises are the analyzer's few built-ins over syntax and names: it cannot read the space's facts.

```notation
; expect: warning
(rule (warning "a to-do written as done already: is that what you meant?")
  (expand (todo ?t title: ?x state: :done list: ?l) in: :head))

(assert (todo todo:bill title: "Pay the bill" state: :done list: list:home))
```

The comment above a rule is kept as data about it, on the branch, and a query reads it like any other fact.

```notation
(select (syntax.meta ?rule description: ?text))
```

A form is data too. When a cell runs, this notebook keeps each of its forms on the branch as the record the notation keeps a tree as (`syntax.form/is`, the "as data" half of the read stage). Dialog's `check` does not write syntax itself; the tour's evaluator does, as Dialog's own tests do. A query then takes the tour's own forms apart by the shape of their trees: every rule written above, by the name its head applies, and every assert of a to-do.

```notation
(select (tour.rule ?f head: ?name)
  (syntax.form ?f is: {:type :application
                       :operator {:type :reference :name /rule}
                       :operand {:items [{:operator {:name ?name}} ... ?premises]}}))

(select (tour.assert ?f to-do: ?t)
  (syntax.form ?f is: {:type :application
                       :operator {:type :reference :name /assert}
                       :operand {:items [{:type :application
                                          :operator {:type :reference :name /todo}
                                          :operand {:items [{:type :constant :value ?t}]}}]}}))
```

## What the notation refuses

Each of these is refused on purpose, and says why and where.

A name nothing defines:

```notation
; expect: refused
(rule (tour.seen ?t)
  (todo ?t title: ?title ...)
  (frobnicate ?t))
```

A line of a concept that says no cardinality:

```notation
; expect: refused
(concept (tour.flag ?t on: ?on)
  (tour.todo/flag of: ?t is: ?on as: :boolean))
```

A read that leaves fields out without `...`: the refusal names what it leaves out and the read written with `...`.

```notation
; expect: refused
(select (todo ?t title: ?title))
```

`top` is gone from the notation: the refusal says what to write instead.

```notation
; expect: refused
(concept (tour.rank ?t level: (top ?u))
  (tour.todo/rank of: ?t is: ?u as: [:high :low] cardinality: :many))
```

A write through a concept that leaves a field out:

```notation
; expect: refused
(assert (todo todo:half title: "Half a to-do"))
```

One rule that both asserts and retracts: it is written as a rule for each.

```notation
; expect: refused
(rule (all (assert (tour.todo ?t list: ?to)) (retract (tour.todo ?t list: ?from)))
  (tour.move ?t to: ?to)
  (tour.todo ?t list: ?from))
```
