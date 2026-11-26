# Findings from the notation tour

Kept as history. Both are fixed at the revision the branch pins now,
`f2bae5bba6548154172828f1b1cc83b1c8ac85b6` (dialog-db `feat/notation-poc`);
finding 2 since `0cebfc07`.
Finding 1 was fixed on the notation branch by b423dd78: the head formula
answers one total per pair. Finding 2 is fixed by dialog-db/dialog-db#863
("a claim one round of induction both retracts and asserts holds
afterwards"): the to-do keeps its title and list and is done. Each
reproduction below now asserts the fixed answer
(`cargo test -p tonk-analyzer --test notation_findings`), and the tour
shows the plain forms.

Two behaviours the tour ran into at the pinned dialog revision
(`7b7484a8cdd8904a2547ae0e2b0c5b935e23151a`, `wip/engine-fold`) and worked
around. Each is checked in as runnable notation under `rust/tonk-analyzer/tests/notation-tour/findings/`, and
`cargo test -p tonk-analyzer --test notation_findings -- --ignored --nocapture --test-threads 1`
runs it through the tour's evaluator (`tonk_analyzer::notation::tour`, which
follows `dialog-notation/examples/notebook/session.rs`) on a fresh branch and
prints what each form gives. The answers below are that output.

## 1. A formula in the head of a select of several premises answers a cross product

`findings/1-head-formula.notation`, one cell, on an empty branch:

```clojure
(concept (estimate ?t hours: ?h)
  (tour.todo/hours of: ?t is: ?h as: :natural cardinality: :one))

(assert (estimate todo:milk hours: 1))
(assert (estimate todo:mum hours: 1))
(assert (estimate todo:deck hours: 3))
(assert (estimate todo:seeds hours: 2))

(select (tour.morning ?a with: ?b total: (+ ?p : ?q))
  (estimate ?a hours: ?p)
  (estimate ?b hours: ?q)
  (< ?p : ?q)
  (<= (+ ?p : ?q) : 4))
```

It answered 8 rows:

```clojure
(tour.morning todo:milk total: 3 with: todo:deck)
(tour.morning todo:milk total: 3 with: todo:seeds)
(tour.morning todo:milk total: 4 with: todo:deck)
(tour.morning todo:milk total: 4 with: todo:seeds)
(tour.morning todo:mum total: 3 with: todo:deck)
(tour.morning todo:mum total: 3 with: todo:seeds)
(tour.morning todo:mum total: 4 with: todo:deck)
(tour.morning todo:mum total: 4 with: todo:seeds)
```

Expected 4, each pair with its own total (milk 1, mum 1, seeds 2, deck 3):

```clojure
(tour.morning todo:milk total: 4 with: todo:deck)
(tour.morning todo:milk total: 3 with: todo:seeds)
(tour.morning todo:mum total: 4 with: todo:deck)
(tour.morning todo:mum total: 3 with: todo:seeds)
```

The rows look like every `total` the premises allow crossed with every
`with:` for the same `?a`: the head's formula is not tied to the row's own
`?p` and `?q`. The tour asks the same pairs without `total:` in the head
(`(select (tour.morning ?a with: ?b) …)`), which answers as expected.

## 2. A retract through a concept retracts every field, also those a rule of the same tick asserts

`findings/2-retract-through-concept.notation`, the first cell, on an empty
branch:

```clojure
(concept (todo ?t title: ?title state: ?state list: ?list)
  (tour.todo/title of: ?t is: ?title as: :text cardinality: :one)
  (tour.todo/state of: ?t is: ?state as: [:open :done] cardinality: :one)
  (tour.todo/list of: ?t is: ?list as: :entity cardinality: :one))

(concept (todo/complete todo: ?t)
  (tour.todo.complete/todo of: ?c is: ?t as: :entity in: :control))

(assert (todo todo:milk title: "Buy milk" state: :open list: list:home))

(rule (assert (todo ?t title: ?title state: :done list: ?l))
  (todo/complete todo: ?t)
  (todo ?t title: ?title state: :open list: ?l))

(rule (retract (todo ?t title: ?title state: :open list: ?l))
  (todo/complete todo: ?t)
  (todo ?t title: ?title state: :open list: ?l))
```

`findings/2b-issue-and-read.notation`, the second cell (the first is its
context, as in the notebook):

```clojure
(assert (todo/complete todo: todo:milk))

(select (todo todo:milk title: ?title state: ?state list: ?l))

(select (tour.todo todo:milk title: ?title))

(select (tour.todo todo:milk state: ?state))

(select (tour.todo todo:milk list: ?list))
```

What it gave:

| select | answers |
|---|---|
| `(todo todo:milk title: ?title state: ?state list: ?l)` | none |
| `(tour.todo todo:milk title: ?title)` | none |
| `(tour.todo todo:milk state: ?state)` | `(tour.todo todo:milk state: :done)` |
| `(tour.todo todo:milk list: ?list)` | none |

So the retract rule took back all three claims of the instance it matched
(title "Buy milk", state open:, list list:home), and the assert rule's title
and list, the same claims asserted in the same tick, did not survive it; only
the state, whose asserted value (`done:`) differs from the retracted one,
stayed.

Expected: the to-do keeps its title and list and is done,
`(todo todo:milk list: list:home state: :done title: "Buy milk")`: the
assert rule writes them again in the tick the retract rule takes them back.
Whether a retract and an assert of the same claim in one tick should leave
it is the question; at this revision the retract wins.

The tour writes the attribute alone instead, which works:

```clojure
(rule (assert (tour.todo/state of: ?t is: :done as: :entity cardinality: :one))
  (todo/complete todo: ?t))

(rule (retract (tour.todo ?t state: ?was))
  (todo/complete todo: ?t)
  (tour.todo ?t state: ?was))
```
