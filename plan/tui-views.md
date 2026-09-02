# TUI views: `show: { tui: … }` rendered in a terminal

## Context

Tonk's view layer is already three-quarters host-independent. This plan
proposes a **third renderer** — a terminal one — alongside the browser
(`tonk-display`) and headless-HTML (`tonk-render`) renderers, driven by a
new `tui` facet of the existing `view` concept:

```yaml
view!:
  this: todo
  show:
    ui:  |          # browser
      <article><h2>{title}</h2></article>
    tui: |          # terminal
      <row><text bold>{title}</text><spacer/><text>{status}</text></row>
```

The point of writing this down first is that **the notation half needs no
design at all**, and the interaction half needs a lot. Section 2 is the
short one on purpose.

## 1. How the existing pipeline is factored

Worth stating precisely, because the factoring is what makes this cheap.

### The `view` concept

`core.yaml` pins `tonk:view`:

```yaml
concept!: &view
  this: tonk:view
  with:
    show:
      the: xyz.tonk.view
      cardinality: one
      as: {[symbol]: text}
```

A view instance's `this` **is the model**. `show` is an open dictionary
keyed by facet; each entry lands as its own fact
`<model> xyz.tonk.view/<facet> <template>` with cardinality one. `ui`,
`directory`, `label`, `title` are conventions, not schema — the type is
`{[symbol]: text}`, so **`tui` is a legal facet today**, with no migration,
no new concept, and no analyzer change.

The route grammar (`tonk-render/src/page/route.rs`) is likewise already
facet-general: `alice@todo!tui` parses today.

### The template language

An HTML fragment with `{field}` holes plus the reserved `{this}`.
`tonk-template` (DOM-free, target-agnostic, no `web-sys`, no target gates)
turns *bindings* into a `BindingPlan`:

- A `Binding` is `{ path: Vec<usize>, kind: Text | Attr }` — a child-index
  path from the fragment root, exactly matching DOM `childNodes` indexing.
- Cardinality-many fields lower to `PlanNode::Iteration`; the iteration
  root is the longest common ancestor of the holes referencing that field.
- The element carrying an attribute whose value is *exactly* `{this}` is
  the **repeat root**: everything outside it is render-once chrome,
  the root itself clones per conclusion.
- Iteration is decided from the *data* at render time (`Ipld::List`
  iterates, a scalar renders once), not hard-coded in the plan.

Two collectors feed one planner: `tonk-display::collect_bindings` walks a
live `DocumentFragment`, `tonk-render::collect_bindings` walks an owned
`Node` tree. Both mutate the tree the same way (splitting interpolated
text nodes into one node per segment) so the `Vec<usize>` paths line up by
construction. **A third collector is not even needed** — a TUI renderer
reuses `tonk-render`'s, because it consumes the same owned tree.

### Resolution

`tonk-render::page::orchestrate` is the host-agnostic resolver:

```
route → model concept (name → URI → phase-1 descriptor)
      → facet (explicit, else `ui` if entity set, else `directory`)
      → view_query(model_entity)  → template
      → entity_query | instances_query → rows
      → fold::select_rows → conclusions
      → collect + plan + render → nested <tonk-display> expansion
```

Its one host seam is a single-method trait:

```rust
#[async_trait(?Send)]
pub trait QueryBackend {
    async fn query(&self, query: ConceptQuery) -> Result<Vec<Conclusion>, RenderError>;
}
```

`tonk` implements it over the on-disk reactor; the worker implements it
over a URL-named branch.

### Rendering

`tonk_render::render_nodes(&roots, &plan, &conclusions) -> Vec<Node>`
produces the **resolved tree** — data substituted, repeats expanded — and
`render()` is just `serialize_nodes(render_nodes(...))`. That boundary is
the whole integration point for a terminal renderer: take the `Vec<Node>`,
never call `serialize`.

### Events and commands

Read path is subscription-driven re-render. Write path is four pieces, and
none of them knows about a browser except step 2:

1. `on<event>=<command>` on a template element names a **transient
   concept**.
2. A delegated listener on the host walks from `event.target` to the
   closest `[data-on<event>]` ancestor, then projects values out of the
   live event object per the command's `with:` map. `the:` identifiers
   under `dom.event.*` are *reads* (`dom.event.current-target.dataset/todo`
   → the bound element's `data-todo`); under `dom.event.do/*` they are
   *side effects* (`preventDefault`). A path that fails to resolve aborts
   the whole assertion.
3. The result is a `TransactRequest` carrying one transient assertion.
4. A `rule!:` matches the transient and asserts durable facts. Rules match
   **structurally** — any transient carrying the command's attribute set
   matches — so the rule never learns what produced the fact.

That last property is the load-bearing one for this plan: **a rule written
for a browser click already works for a terminal keypress**, provided the
terminal host posts a transient of the same shape.

## 2. What transfers for free

| Piece | Status |
| --- | --- |
| `view` concept / `show` dictionary | `tui` is a legal facet today |
| Route grammar (`entity@model!tui`) | works today |
| `tonk-template` planner, `resolve`, `fold` | DOM-free by construction |
| `tonk-render::{parse, tree, collect, render_nodes}` | tag-name-agnostic; only `is_void_tag` / `is_raw_text_element` are HTML-specific |
| Reactor `branch.subscribe(ConceptQuery)` | native, no service worker |
| Commands, transients, rules | host-independent; the reactor never sees the event |

The `html5gum` tokenizer parses `<row><text bold>{title}</text></row>`
today. So the *authoring* story — write a template, get a plan, get a
resolved tree — needs zero new code.

## 3. What actually has to be built

Five things, in rough order of risk:

1. **An interaction model with no pointer and no focus manager.** (§4)
2. **Terminal event → transient extraction.** (§5)
3. **An element vocabulary + layout engine, `Node` tree → ratatui.** (§6)
4. **A subscription seam** — `QueryBackend` is one-shot. (§7)
5. **A default `tui` facet** — the `tonk:_` analogue. (§8)

Note the ordering. The component list is the *last* interesting problem,
not the first.

## 4. The interaction model — the real design surface

In a browser the platform supplies: a pointer, hit-testing, a focus ring,
tab order, text-input carets, scroll containers, and `:focus`/`:hover`
styling. In a terminal we supply all of it. Proposal:

- **Focusable** = any element carrying an `on<event>` attribute, plus any
  element with an explicit `focus` attribute. Document order is tab order.
- **Traversal**: `Tab` / `Shift-Tab` always; arrow keys within a container
  that declares `nav=vertical|horizontal`.
- **Activation**: `Enter` and `Space` on a focused element fire its
  `onclick`. Mapping activate→`onclick` is deliberate: it keeps browser
  commands reusable verbatim rather than forcing every app to author an
  `onactivate` twin.
- **Mouse**: crossterm gives real clicks; a click also fires `onclick`,
  with `tui.event/row` / `tui.event/column` available for commands that
  want them.
- **Focus styling**: no CSS, so no `:focus` selector. Focused elements get
  a renderer-drawn indicator by default, overridable per element with
  `focus-fg` / `focus-bg` / `focus-border` attributes.
- **Scrolling**: a `<scroll>` container owns an offset and consumes
  `PageUp`/`PageDown`/wheel. This is host state, not branch state (§4.1).

### 4.1 Widget-local state has nowhere to live

The template is data and the plan is rebuilt per frame; there is no
component instance to hang a caret position or a scroll offset on. Two
options:

- **(a) Host-side state map**, keyed by `(repeat-root conclusion `this`,
  binding path)`. This is what the DOM does for you. Boring, correct,
  survives a re-render as long as the key is stable.
- **(b) Session-overlay facts.** `branch.overlay()` already exists: writes
  land in an in-memory overlay, are never committed and never replicated,
  and **subscriptions still see them**. Caret position as an overlay fact
  means a template could bind `{cursor}` and rules could react to it.

(b) is elegant and very on-model, but costs a reactor round-trip per
keystroke and makes every input a distributed-state problem. **Recommend
(a) for the first implementation**, with (b) noted as the interesting
follow-on for state that genuinely wants to be queryable (selection,
active tab, expanded rows) rather than for carets.

## 5. Event namespace: reuse `dom.event.*` or fork?

Options:

- **(a) Reuse `dom.event.*` verbatim** for everything that maps 1:1, and
  add `tui.event/*` only for terminal-only reads.
- **(b) A parallel `tui.event.*` namespace.**
- **(c) A neutral `ui.event.*` both hosts implement, `dom.event.*` aliased.**

**Recommend (a).** The structural paths map cleanly onto a terminal:

| `the:` identifier | terminal meaning |
| --- | --- |
| `dom.event.current-target.dataset/todo` | the `data-todo` attribute on the focused/activated node |
| `dom.event.target.dataset/*` | the innermost node under the activation |
| `dom.event.current-target.form.elements.<name>/value` | the named input inside the enclosing `<form>` subtree |
| `dom.event/key` | the pressed key |
| `dom.event/type` | `click`, `key`, `change`, `submit` |
| `dom.event.detail/*` | payload of a widget-raised event |
| `dom.event.do/prevent-default` | no-op (nothing native to prevent) |
| — | `tui.event/row`, `tui.event/column`, `tui.event/modifiers` |

Forking the namespace (b) forks every command and every rule, doubling the
application for no semantic gain. (c) is the honest naming but is a
migration across the whole standard library; take it later if the `dom`
misnomer becomes a real teaching problem, not now.

**Consequence to state loudly:** the existing "one command, one shape"
hazard gets sharper with two hosts. A browser `onclick` command and a
terminal activation command that read the same attributes are *the same
shape* and fire the same rules. That is arguably the correct semantics —
the rule expresses host-independent intent — but the analyzer's
subset-overlap check now spans hosts, and an author debugging "why did the
terminal fire the web rule" needs to be told this is by design.

`dom.event.do/prevent-default` becoming a no-op has a nasty edge: the
prevent-default trap (an action field makes a command rule-proof) still
applies in the TUI even though nothing is being prevented, because the
field still stores no value and a rule premise over it still matches zero
rows. **Recommend the TUI host warn** when a command it fires declares a
`dom.event.do/*` field.

## 6. Element vocabulary and layout

### Syntax

Keep HTML syntax. Non-negotiable in my view: it buys the parser, the
collector, the planner, `{field}`/`{this}`, and nested `<tonk-display>`
for free, and an author who knows the `ui` facet can read the `tui` one.

### Tag set

Two candidates:

- **(a) Reuse HTML tag names** with terminal semantics: `<div>`→block,
  `<span>`→inline, `<ul>/<li>`→list, `<table>`→table, `<button>`,
  `<input>`, `<form>`.
- **(b) A distinct vocabulary**: `<box>`, `<row>`, `<column>`, `<text>`,
  `<list>`, `<table>`, `<input>`, `<tabs>`, `<gauge>`.

**Recommend (b) for layout and terminal-native widgets, with unknown tags
degrading to "block containing children" rather than erroring** — and
explicitly *not* chasing "one template serves both facets."

The temptation with (a) is template reuse. It is a trap: terminal layout
is a fixed cell grid with no reflow, no overflow-scroll for free, and no
inline text wrapping subtleties. A template that satisfies both will be
bad at both. The `tui` facet exists precisely so they can differ. Reuse
the *syntax* and the *pipeline*, not the *template*.

Two exceptions where the HTML name must be kept because the pipeline
depends on it:

- **`<tonk-display>`** — the composition and cross-concept-join primitive.
  `<tonk-display entity={author} model=person view=label>` must work
  identically, so `label` facets are shared between hosts (they are just
  text — a fine thing to share).
- **`<tonk-fallback>`** — the empty-state affordance, keyed off the host's
  `data-state`.

### Layout

**Expose ratatui's constraint vocabulary directly; do not invent one.**
`Constraint::{Length, Percentage, Min, Max, Fill}` is already the right
model. Attributes, not CSS:

```
<column gap=1 pad=1>
  <row height=1><text bold>{title}</text><spacer/><text dim>{count}</text></row>
  <scroll grow=1>
    <list subject={this}>
      <row><text>{title}</text></row>
    </list>
  </scroll>
</column>
```

`<style>` blocks are ignored — the collector already skips them as
raw-text elements, so this costs nothing. A CSS subset would need a
selector engine and a cascade; that is an unbounded commitment for a
surface whose whole styling need is roughly `fg`, `bg`, `bold`, `dim`,
`underline`, `border`.

### Candidate component set

**This list is provisional.** It should be pinned against
`tonk-labs/gooey@mvp:tui/showcase.html`, which this session cannot read
(the repo is not accessible to it). Until then:

- Layout: `<row>`, `<column>`, `<box>` (bordered/titled), `<spacer>`,
  `<scroll>`
- Text: `<text>` (fg/bg/bold/dim/underline), `<p>` (wrapping)
- Collections: `<list>`, `<table>` (with `<thead>`/`<tbody>` chrome, the
  `{this}` repeat root on the row)
- Input: `<input>`, `<textarea>`, `<checkbox>`, `<select>`, `<form>`
- Chrome: `<tabs>`, `<gauge>`, `<sparkline>`, `<spinner>`, `<callout>`
- Tonk: `<tonk-display>`, `<tonk-fallback>`

## 7. The subscription seam

`QueryBackend::query` is one-shot; `tonk render` needs nothing more. The
TUI needs a stream. Add a **separate** trait rather than widening the
existing one, so the `tonk render` path is untouched:

```rust
#[async_trait(?Send)]
pub trait SubscribeBackend: QueryBackend {
    fn subscribe(&self, query: ConceptQuery) -> BoxStream<'_, Result<Vec<Conclusion>, RenderError>>;
}
```

The reactor already provides `branch.subscribe(query)` natively, so the
`tonk` host implements it directly against the on-disk `.tonk/` — no
service worker, no wasm, no HTTP.

### The one refactor this plan requires

`orchestrate::render` returns a `String` and expands nested
`<tonk-display>` elements by re-rendering *into the HTML string*.
`render_portal` likewise emits an `<iframe srcdoc>`. Both are
HTML-specific.

Proposal: split orchestration so the shared half returns the resolved
`Vec<Node>` and does nested expansion **on the tree**, with serialization
and portal handling as an HTML-only tail. `tonk render` keeps its exact
current output; the TUI consumes the tree. This is a real refactor with
real regression risk (nested expansion is where the recursion guard and
the visited-set cycle detection live), and is the single largest piece of
work in the "free" column that is not actually free.

Portals (`type: text/html`) have no terminal meaning. **Recommend the TUI
render a placeholder box naming the portal**, not an error.

## 8. The default `tui` facet, and who authors these

If the terminal is meant to be a peer surface for every space, somebody
has to author a `tui` facet for every model — or the feature is dead on
arrival for every existing space.

The browser solves this with `view!: { this: tonk:_, show: { directory: … } }`
— a wildcard-model fallback that renders any model's instances in a
carousel of nested single-entity displays. **The TUI needs the same
thing**: a `tonk:_` `tui` facet (a generic table or list of instances,
each row a nested `<tonk-display view=label>`), plus the existing notation
fallback for single entities.

That gives every space a usable terminal view with zero authoring, and
makes the hand-written `tui` facet an upgrade rather than a prerequisite.
**This is the difference between a demo and a feature** and should land in
the first milestone that has anything interactive, not last.

## 9. Where the code lives

- **`rust/tonk-tui`** (new, native-only): element vocabulary, layout,
  ratatui paint, focus model, terminal-event → transient extraction.
  Depends on `tonk-template`, `tonk-render` (parse/tree/collect/
  `render_nodes`), `tonk-schema`, `ratatui`, `crossterm`.
- **`rust/tonk-render`**: the orchestration split from §7. No new deps.
- **`rust/tonk-cli`**: `tonk tui [route]` in `src/tui.rs`, implementing
  `SubscribeBackend` over `TonkSite`'s reactor (mirroring the existing
  `QueryBackend for TonkSite` impl in `src/render.rs`).

Do **not** pre-emptively split `tonk-render` into a tree crate and an HTML
crate. `tonk-tui` depending on `tonk-render` and never calling
`serialize_nodes` is fine until it isn't.

`ratatui` is already a workspace dependency in the sibling `dialog-db`
workspace (`dialog-diagnose`), so the stack has precedent.

## 10. Ratatui vs. an ink-style reconciler

Ink's value proposition is a reconciler for a component tree authored in
JS. Tonk has no component tree in the host: templates are branch data,
plans are rebuilt per frame, and the retained state lives in the reactor.
A reconciler solves a problem this architecture does not have.

**Recommend immediate-mode ratatui**: rebuild the widget tree from the
resolved `Vec<Node>` each frame. Data changes arrive at subscription
cadence (not 60fps), and the browser renderer's incremental DOM diffing
exists because the DOM is expensive to rebuild — a cell buffer is not.

The cost is §4.1: no per-widget instance state. That is a real cost and
the reason §4.1 needs an answer before §6 gets interesting.

## 11. Milestones

- **M0 — static frame.** `tonk tui <route>` resolves a route, renders one
  frame, exits on `q`. Vocabulary: `<row>`, `<column>`, `<box>`, `<text>`,
  `<list>`, `<table>`, `<spacer>`. No events, no focus. Proves
  parse → plan → `render_nodes` → ratatui end to end.
- **M1 — live.** `SubscribeBackend` over the reactor; redraw on frame
  change. Empty/loading/error states mapped onto the existing `State`
  enum. The `tonk:_` `tui` fallback from §8.
- **M2 — activation.** Focus ring, tab traversal, `Enter`/`Space` →
  `onclick` → transient → transact. This is where the browser's rules
  start firing from a terminal.
- **M3 — input.** `<input>`, `<textarea>`, `<checkbox>`, `<select>`,
  `<form>`; `onchange` / `onsubmit`; host-side widget state per §4.1(a).
- **M4 — chrome.** `<scroll>`, `<tabs>`, `<gauge>`, `<sparkline>`, mouse,
  nested `<tonk-display>` composition, `<tonk-fallback>`.

## 12. Testing

The repo already treats browser/headless parity as a first-class concern
(`plan/view-anchor-render-parity.md`). A third renderer makes that a
three-way problem, but only for the shared half — the planner. Proposal:

- **Plan parity**: assert `tonk-tui` and `tonk-render` produce identical
  `BindingPlan`s for the same template. Free, since they share the
  collector.
- **Paint snapshots**: ratatui's `TestBackend` renders to a fixed-size
  cell buffer; snapshot it. This is the TUI analogue of the existing HTML
  string assertions.
- **Command parity**: assert the transient a terminal activation posts is
  byte-identical to the one a browser click posts for the same command
  descriptor and the same `data-*`. This is the claim §5 rests on; test it
  rather than assume it.

## 13. Open questions

1. **`tonk-labs/gooey@mvp:tui/showcase.html`** — needed to pin §6's
   component list. Not readable from this session.
2. **Is the terminal a peer surface for every space, or a TUI-first
   authoring/inspection tool?** §8's answer changes completely depending
   on which. If it is a peer surface, the `tonk:_` fallback is
   load-bearing and M1-critical. If it is an inspection tool, a small
   hand-authored set of `tui` facets on the standard library models may be
   enough and §8 shrinks to nothing.
3. **Does `tui` want sub-facets?** A terminal has modes a browser does not
   (a compact status line vs. a full pane). `show: { tui: …, tui-line: … }`
   is free in the schema; whether it is a good idea is a question about how
   many facets an author can hold in their head.
4. **Does the analyzer need to know about `tui`?** Today it validates
   `on<event>` targets against transient descriptors. If TUI-only events
   (`onkey`) exist, it needs the vocabulary — or it needs to stop caring
   about which events are legal.
