# Event handling

Status: design. Defines how DOM events on rendered tonk-display markup become transient concept assertions that cross the service-worker boundary, without introducing a new notation form or a separate rule kind. Rule-driven event extraction (rules that run on main thread, conditional side effects) is acknowledged but out of scope.

## Complication

Tonk-display renders DOM on the main thread. The reactor runs in a service worker. DOM events exist only on the main thread. Rules (and the dialog engine generally) run only in the service worker. So a click on a rendered element can't directly trigger a rule — the event object never crosses to the worker.

What does cross is the `/transact` request body: a `TransactRequest` carrying a typed `Claim` batch. Events have to be converted, on the main thread, into one of those.

The question this document answers: how is that conversion declared, and how does the runtime perform it.

## The model

A concept attribute is a named field at the user-facing layer paired with `the:` — the identifier of the relation under which the field's value actually lives. Dialog storage is the usual home for those relations: writing a fact emits an EAV whose attribute identifier is the `the:` value, reading walks matching EAVs back into the named fields. The same pairing carries the field both directions across the same boundary.

On main thread there is no dialog store; the event itself stands in for it. A DOM event has paths through it — `event.type`, `event.shiftKey`, `event.target.dataset.counter` — and the runtime treats each path as something a `the:` identifier can resolve to. When a concept's `the:` identifiers name paths into the event, the runtime can walk the concept's `with:` map, follow each one into the live event, coerce the result through `as:`, and assemble a `Claim::Assert(PredicateApplication { ... })`. That claim posts to the worker as a regular transaction.

So nothing new happens at the concept level. The `the:` identifier does the same job it always does — name where the value is — only the "where" is the event instead of storage. A namespace convention (`dom.event` and its sub-paths) signals which `the:` identifiers the main-thread runtime should follow into the event rather than against dialog. Concepts addressed elsewhere can't be filled this way; they need a rule in between (future work).

The template tells the runtime which concept to assemble. `<button onclick=increment>+</button>` reads as: when this element fires a click, produce an `increment` concept from the event. The runtime resolves `increment`, does the walk, posts the resulting `Claim`. The worker handles the rest the same way it handles any transient.

The split is clean:

- **Concept author** decides what the asserted fact contains (its attributes and types).
- **Concept attribute** authors decide where each field comes from (its `the:` identifier), inside or outside the `dom.event` namespace.
- **Template** says which concept a given DOM event produces.
- **Main-thread runtime** does the projection and posts.
- **Service worker** processes the resulting transient like any other.

Nothing in the analyzer changes. Nothing in dialog changes. Nothing in the rule machinery changes. The whole feature lives in:

1. A namespace convention for `the:` identifiers.
2. A small main-thread runtime that walks a concept's attributes and projects values from a JS event.
3. A template attribute that names the target concept.

## Template syntax

```html
<button onclick=increment data-counter={this}>+</button>
```

`onclick=<concept>` names the concept the click produces. The concept can be named (`increment`) or referenced by entity URI (`onclick="did:key:zinc..."`). Names are a convenience for resolution; entity URIs avoid the global-name dependency.

A bare `onclick` without a concept is not allowed. The main-thread runtime needs to know which concept's schema to walk; without that, there's no extraction to perform and no obvious default that wouldn't paint us into a corner.

One `on<event>` attribute per event type per element. If two clicks should produce two different facts, write two elements. Multi-binding to one element is out of scope.

Other event attributes follow the same pattern: `onkeydown=submit`, `onpointerdown=select`, `onsubmit=save`, etc. The event name is whatever the DOM type defines; the runtime just reads the matching event off the element.

## The `dom.event` namespace

A `the:` identifier under the `dom.event` prefix is read by the main-thread runtime as a path into the JS event. The path crosses both `/` and `.` in the identifier as property steps on the event, and kebab-case segments become camelCase along the way.

Examples:

```
dom.event/type                 → event.type
dom.event/pressure             → event.pressure
dom.event/key                  → event.key
dom.event/shift-key            → event.shiftKey
dom.event/client-x             → event.clientX
dom.event.target/value         → event.target.value
dom.event.target.dataset/counter → event.target.dataset.counter
dom.event.target.dataset/item-id → event.target.dataset.itemId
```

The translation, stated plainly:

- The `dom` segment is the root of the namespace; `event` is the binding to the JS event object the handler received.
- Subsequent segments — split on either `.` or `/` — are successive property accesses against that object.
- Dashed identifiers within a segment become camelCase before lookup: `shift-key` reads as `shiftKey`, `item-id` as `itemId`.

The runtime evaluates each path against the JS event object live, at handler-fire time. If a path doesn't resolve (e.g. `event.pressure` is undefined for a `MouseEvent`), the corresponding attribute is omitted from the asserted parameters. The schema's `as:` type determines coercion: `text` → string, `entity` → parse as a URI, `float` → number, etc.

This is one-way for now — reading. The bidirectional case (concept attributes that *write* to the event, like `preventDefault`) is described below.

### Context via data attributes

The entity a view is rendering and its other bound fields don't reach the click handler implicitly. They reach it through `data-*` attributes the template author writes on the element, using the template's existing `{field}` interpolation:

```yaml
view!:
  template: |
    <button onclick=increment data-counter={this}>+</button>
```

The concept then reads the attribute back via `dom.event.target.dataset/counter`:

```yaml
concept!: &increment
  transient:
  with:
    counter:
      the: dom.event.target.dataset/counter
      as: entity
```

Explicit, no implicit injection of view context, no path-traversal magic. The template author decides which values to surface on the element; the concept author decides which to read. Both have to agree on the `data-<name>` they share. The chain `data-counter={this}` → `event.target.dataset.counter` → `?counter` (the variable the rule body binds) is visible at every step.

The downside is verbosity for the common case (every click on an entity-bearing element needs an explicit `data-*` attribute). The upside is that it works without any new injection rule — the existing template `{field}` interpolation writes the value into a `data-*` attribute, and the existing `the:` machinery reads it back.

## Side effects: outgoing attributes

`preventDefault()` and `stopPropagation()` are calls on the JS event, not properties of it. They take effect only synchronously within the original event handler's tick. By the time the worker responds to a `/transact`, the event is gone.

So they belong on the *immediately asserted* concept — the one the click handler produces — not on a downstream rule's head. The main-thread runtime, while constructing the assertion, also walks the concept's attributes for the "do" namespace and calls the corresponding methods on the event:

```
dom.event.do/prevent-default     → event.preventDefault()
dom.event.do/stop-propagation    → event.stopPropagation()
dom.event.do/stop-immediate-propagation → event.stopImmediatePropagation()
```

These are unconditional. The attribute's presence in the concept's `with:` schema is the signal — the main-thread runtime sees a `the:` identifier under `dom.event.do` and calls the corresponding method on the JS event. The schema entry doesn't carry a value to read; we don't have conditionals at concept-declaration time, so presence is enough.

```yaml
concept!: &select
  transient:
  with:
    counter:
      the: dom.event.target.dataset/counter
      as: entity
    prevent-default:
      the: dom.event.do/prevent-default
```

The runtime, walking `select`'s schema while building an assertion, sees `dom.event.do/prevent-default` and calls `event.preventDefault()` before the handler returns. The resulting `Claim` carries only the read-side attributes (`counter`); action attributes don't appear in the asserted fact because there's no value to put there.

## What the main-thread runtime does

On each rendered element with `on<event>=<concept>`, the runtime calls `addEventListener` with the event name and a handler. The runtime does not validate the event name — whatever string the template supplied is the listener type. On handler fire:

1. Resolve the named concept's `with:` map.
2. For each attribute:
   - If `the:` sits under `dom.event` but outside `dom.event.do`, evaluate the path against the JS event and coerce via `as:`.
   - If `the:` sits under `dom.event.do`, call the corresponding method on the event.
3. Build a `Claim::Assert(PredicateApplication { predicate, parameters })` from the read-side values.
4. Wrap in a `TransactRequest { claims: vec![claim] }`.
5. Post to `/api/repository/{repo}/branch/{branch}/transact`.

No state on main thread beyond the per-render binding table. No reactor, no induction loop, no rule machinery. The event is converted and forwarded.

The wire payload is an ordinary `TransactRequest` with one transient claim. The worker's `/transact` route handles it identically to any other typed transact. Nothing on the worker side needs to know the claim originated from a DOM event.

## Examples

### A click that increments a counter

```yaml
concept!: &counter
  with:
    count: { the: xyz.tonk.counter/count, as: integer }

concept!: &increment
  transient:
  with:
    counter: { the: dom.event.target.dataset/counter, as: entity }

rule!:
  when:
    - assert: increment
      where: { counter: ?counter }
    - assert: counter
      where: { this: ?counter, count: ?count }
  assert!: counter
  where:
    this: ?counter
    count: ?count + 1
```

```html
<button onclick=increment data-counter={this}>+</button>
```

The click asserts an `increment` transient with `counter` bound to the button's `data-counter` (which the template populated from the view's `{this}`). The rule on the worker reads the `increment` along with the current `counter` fact, fires once per matching counter, asserts the new count. Transient `increment` sweeps before the durable commit.

### Form submission

```yaml
concept!: &save
  transient:
  with:
    title: { the: dom.event.target.dataset/title, as: text }
    body:  { the: dom.event.target/value,         as: text }
    prevent-default: { the: dom.event.do/prevent-default }
```

```html
<form onsubmit=save data-title="Untitled">
  <textarea name="body"></textarea>
  <button type="submit">Save</button>
</form>
```

The submit asserts a `save` with the form's `data-title` and the textarea's value. `preventDefault` is called synchronously so the page doesn't reload. The worker handles the rest.

## What's out of scope

- **Rule-driven extraction.** A rule whose body reads from a hypothetical `event` formula and asserts a concept of the caller's choice. Coherent with this design (concepts are still concepts, rules are still rules) but requires the `event` formula's signature and the rule-loading semantics on main thread to be designed. Not now.
- **Multi-binding per element.** One `on<event>` per event type per element. Multiple events on one element are fine; multiple concepts on the same event are not.
- **Conditional side effects.** `preventDefault` is on or off at concept-declaration time. Conditional behavior requires the rule-driven extension.
- **Multi-head rules.** Not introduced here, not introduced anywhere. If a transient should fan out to multiple downstream facts, write multiple rules.
- **`getTargetRanges()` / complex DOM APIs.** Anything that needs synchronous JS API access beyond reading event properties belongs in a custom web component the template embeds.

## Migration

This is a strictly additive feature. No existing notation changes. No existing concepts change. No analyzer rules change. The new pieces are:

1. The main-thread runtime's bindings table and extraction loop.
2. The `dom.event` namespace convention (with `dom.event.do` as its action-attribute sub-namespace), documented but not enforced anywhere — these are regular `the:` identifiers the main-thread runtime happens to recognize.
3. The `on<event>=<concept>` template attribute parsing.

Concepts whose attributes are addressed outside `dom.event` are unaffected. Templates that don't use `on<event>` attributes are unaffected. Workers don't need to distinguish event-derived assertions from any other — the wire format is the same `TransactRequest` shape.
