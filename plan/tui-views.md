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
      <row><text ink=bold>{title}</text><spacer/><text ink=dim>{status}</text></row>
```

Two things make this cheaper than it looks and one makes it more
opinionated:

- The **notation half needs no design at all** — `tui` is already a legal
  facet (§1.1), and `alice@todo!tui` already parses.
- The **pipeline is already DOM-free** down to a `Vec<Node>` seam (§1.4).
- The **visual language is already decided** — `stripes` (§2) fixes the
  style vocabulary at five ink treatments with no color, which shrinks the
  attribute surface dramatically and answers several open questions
  outright.

What is left is the interaction model (§5), which is where all the risk
is.

## 1. How the existing pipeline is factored

### 1.1 The `view` concept

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
`{[symbol]: text}`, so **`tui` is a legal facet today**: no migration, no
new concept, no analyzer change. The route grammar
(`tonk-render/src/page/route.rs`) is likewise facet-general.

### 1.2 The template language

An HTML fragment with `{field}` holes plus the reserved `{this}`.
`tonk-template` (DOM-free, target-agnostic, no `web-sys`, no target gates)
turns *bindings* into a `BindingPlan`:

- A `Binding` is `{ path: Vec<usize>, kind: Text | Attr }` — a child-index
  path from the fragment root, matching DOM `childNodes` indexing.
- Cardinality-many fields lower to `PlanNode::Iteration`; the iteration
  root is the longest common ancestor of the holes referencing that field.
- The element carrying an attribute whose value is *exactly* `{this}` is
  the **repeat root**: everything outside it is render-once chrome, the
  root itself clones per conclusion.
- Iteration is decided from the *data* at render time (`Ipld::List`
  iterates, a scalar renders once), not hard-coded in the plan.

Two collectors feed one planner: `tonk-display::collect_bindings` walks a
live `DocumentFragment`, `tonk-render::collect_bindings` walks an owned
`Node` tree. Both mutate their tree identically (splitting interpolated
text nodes into one node per segment) so the `Vec<usize>` paths line up by
construction. **A third collector is not needed** — a TUI renderer reuses
`tonk-render`'s, because it consumes the same owned tree.

### 1.3 Resolution

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

`tonk` implements it over the on-disk reactor; the worker over a
URL-named branch.

### 1.4 The rendering seam

`tonk_render::render_nodes(&roots, &plan, &conclusions) -> Vec<Node>`
produces the **resolved tree** — data substituted, repeats expanded — and
`render()` is just `serialize_nodes(render_nodes(...))`. That boundary is
the integration point: take the `Vec<Node>`, never call `serialize`.

### 1.5 Events and commands

Read path is subscription-driven re-render. Write path is four pieces, and
only step 2 knows about a browser:

1. `on<event>=<command>` on a template element names a **transient
   concept**.
2. A delegated listener on the host walks from `event.target` to the
   closest `[data-on<event>]` ancestor, then projects values out of the
   live event per the command's `with:` map. `the:` identifiers under
   `dom.event.*` are *reads*
   (`dom.event.current-target.dataset/todo` → the bound element's
   `data-todo`); under `dom.event.do/*` they are *side effects*. A path
   that fails to resolve aborts the whole assertion.
3. The result is a `TransactRequest` carrying one transient assertion.
4. A `rule!:` matches the transient and asserts durable facts. Rules match
   **structurally** — any transient carrying the command's attribute set
   matches — so the rule never learns what produced the fact.

That last property is load-bearing: **a rule written for a browser click
already works for a terminal keypress**, provided the terminal host posts
a transient of the same shape.

## 2. `stripes` — the design system, and what it actually covers

Source: `tonk-labs/gooey@mvp:tui/showcase.html` (v0.2), with
`tui/tonk-tui.md` (spec), `tui/README.md` (rationale), `tui/demo.sh`
(living style guide) and `tui/tokens.json` (machine-readable tokens)
alongside it. Only the showcase has been read for this plan; the other
three should be read before implementation.

### 2.1 The correction this forces

**`stripes` is a CLI *output* design system, not a widget framework.** Its
components are numbered against `tui/demo.sh` sections and are the
surfaces of a non-interactive command: log lines, sync-state glyphs,
prompts, progress, tables, blocks, `USAGE`/`COMMANDS` help, keybar. It
addresses `tonk eval` / `tonk schema` / `--version` printing to stdout.

That is a *different renderer* from a `tui` view facet, which paints
branch data into a full-screen alternate buffer. Conflating them would be
a mistake. The right relationship:

- **`stripes` supplies the visual language** — ink treatments, glyphs,
  motion timings, degradation rules — to both surfaces.
- **`stripes` does not supply the widget vocabulary** for `tui` views,
  because a view renders *user data*, not tool chrome. Its `08 · table`
  and `09 · blocks` panes are the only components that transfer directly.

So the showcase **pins the style budget and leaves the widget set open** —
which is the good outcome, because the style budget is where an
un-opinionated renderer would have sprawled.

Note also that `tonk-cli` today emits plain text with **no styling crate
at all** (`output.rs` writes strings; the workspace has no
`owo-colors`/`anstyle`/`crossterm`). Stripes is unimplemented on both
surfaces. Whoever lands the ink primitives first should land them as a
shared crate (§9), not inside either renderer.

### 2.2 The laws, and what each one costs the renderer

| Law | Renderer consequence |
| --- | --- |
| **ink only** — no color codes exist; hierarchy is weight | The style attribute has **five values**, not a color space. Kills `fg`/`bg`/`focus-fg` entirely. |
| **alerts blink, never color** — reverse wash on a 2.4 s calm cycle; interaction calms it; SGR blink forbidden | The renderer needs an **animation clock independent of data change** (§6.3). This is the single most-missed requirement. |
| **frost is the surface** — washes make chips and selections; plate (full reverse) is the CTA | **Answers the focus-styling question outright**: focus = frost, primary/armed = plate. No focus-style attributes needed. |
| **fixed cells** — chips never move or resize while visible; hard corners `┌┐`, never `╭╮` | Chips are `Constraint::Length`, never `Fill`. Rules out reflowing a keybar on resize. |
| **the stripe is the brand** — horizontal bars (`▀`) carry identity | Progress is one stripe of the mark (bold fill / dim track). Never vertical. |
| **lowercase chrome** — tool words lowercase, user words untouched | A **renderer rule**: vocabulary-owned labels lowercase; `{field}` interpolations pass through verbatim. Worth enforcing in the element set rather than trusting authors. |

### 2.3 The ink vocabulary — the whole style surface

| Treatment | SGR | Role |
| --- | --- | --- |
| bold | `1` | heads, emphasis, the mark, alert text |
| plain | — | body (terminal default foreground) |
| dim | `2` | meta, tracks, hairlines, disabled |
| frost | `2;7` | quiet chips, **selection / focus washes** |
| plate | `7` | CTAs, alerts, **the selected option** |

Owned surfaces: light `#34332b` on `#fbfaef`; dark `#e9e6d6` on `#21211b`.
Type: terminal's own mono for content; IBM Plex Sans Condensed 600 for
chrome tonk controls. Never italic, never underline (except links), never
SGR blink.

### 2.4 Motion budget

- 2.4 s **calm cycle** — everything repeating breathes at this rate
- spinner: 8 frames × 300 ms = one pass per calm cycle
- progress repaints ≤ 10 fps
- alert pulse: reverse wash 0→14 % over one calm cycle; interaction calms it
- block cursor: 1.05 s hard blink (`steps`)
- \> 400 ms gets a spinner; > 3 s gets a progress bar or log stream

### 2.5 Degradation matrix

| Condition | Behaviour |
| --- | --- |
| `NO_COLOR` | already satisfied — the system emits no color |
| no dim support | dim → plain; frost chips → `[bracketed]` text |
| non-UTF-8 | `▀`→`#` (tracks `.`) · `●◐○`→`*o.` · box→`+-|` · logo → bold `tonk cli` |
| not a tty | no SGR, no spinner, no cursor writes; plain log lines |
| < 80 cols | lockup instead of banner; bars shrink to 12 cells |

This is a **renderer responsibility, not an author responsibility**, and
it is unusually testable (§12).

### 2.6 `tokens.json` is the source of truth

`tui/tokens.json` carries ink treatments, glyphs, spinner frames and logo
bitmaps machine-readably. **Consume it; do not retype it.** Either vendor
it into the ink crate as a build-time asset or mirror it with a parity
test. Hand-transcribed glyph tables drift.

## 3. What transfers for free

| Piece | Status |
| --- | --- |
| `view` concept / `show` dictionary | `tui` is a legal facet today |
| Route grammar (`entity@model!tui`) | works today |
| `tonk-template` planner, `resolve`, `fold` | DOM-free by construction |
| `tonk-render::{parse, tree, collect, render_nodes}` | tag-name-agnostic; only `is_void_tag` / `is_raw_text_element` are HTML-specific |
| Reactor `branch.subscribe(ConceptQuery)` | native, no service worker |
| Commands, transients, rules | host-independent; the reactor never sees the event |

The `html5gum` tokenizer parses `<row><text ink=bold>{title}</text></row>`
today. The *authoring* story — write a template, get a plan, get a
resolved tree — needs zero new code.

## 4. What has to be built

1. An **interaction model** with no pointer and no focus manager (§5)
2. **Terminal event → transient extraction** (§5.3)
3. An **element vocabulary + layout + stripes paint** (§6)
4. A **subscription seam** — `QueryBackend` is one-shot (§7)
5. A **default `tui` facet** — the `tonk:_` analogue (§8)

The component list is the *last* interesting problem, not the first.

## 5. The interaction model — the real design surface

A browser supplies a pointer, hit-testing, a focus ring, tab order, text
carets, scroll containers, and `:focus`/`:hover` styling. A terminal
supplies none of it.

### 5.1 Focus and activation

- **Focusable** = any element carrying an `on<event>` attribute, plus any
  element with an explicit `focus` attribute. Document order is tab order.
- **Traversal**: `Tab` / `Shift-Tab` always; arrow keys within a container
  declaring `nav=vertical|horizontal`.
- **Activation**: `Enter` and `Space` on a focused element fire its
  `onclick`. Mapping activate→`onclick` is deliberate — it keeps browser
  commands reusable verbatim rather than forcing an `onactivate` twin.
- **Focus styling is settled by law 3**: focused = **frost**, armed /
  primary = **plate**. No `focus-*` attributes. A destructive key pulses
  while armed (calm cycle) and never recolors.
- **Mouse**: crossterm gives real clicks; a click also fires `onclick`,
  with `tui.event/row` / `tui.event/column` available.
- **Scrolling**: a `<scroll>` container owns an offset and consumes
  `PageUp`/`PageDown`/wheel. Host state, not branch state (§5.2).

### 5.2 Widget-local state has nowhere to live

The template is data and the plan is rebuilt per frame; there is no
component instance to hang a caret position or scroll offset on.

- **(a) Host-side state map**, keyed by `(repeat-root conclusion `this`,
  binding path)`. What the DOM does for you. Boring, correct, survives
  re-render while the key is stable.
- **(b) Session-overlay facts.** `branch.overlay()` already exists: writes
  land in an in-memory overlay, are never committed and never replicated,
  and **subscriptions still see them**. Caret position as an overlay fact
  means a template could bind `{cursor}` and rules could react.

(b) is elegant and on-model but costs a reactor round-trip per keystroke.
**Recommend (a) first**, with (b) as the follow-on for state that
genuinely wants to be queryable — selection, active tab, expanded rows —
never for carets.

### 5.3 Event namespace: reuse `dom.event.*` or fork?

- **(a) Reuse `dom.event.*`** for everything that maps 1:1; add
  `tui.event/*` only for terminal-only reads.
- **(b) A parallel `tui.event.*` namespace.**
- **(c) A neutral `ui.event.*` both hosts implement, `dom.event.*` aliased.**

**Recommend (a).** The structural paths map cleanly:

| `the:` identifier | terminal meaning |
| --- | --- |
| `dom.event.current-target.dataset/todo` | the `data-todo` on the focused/activated node |
| `dom.event.target.dataset/*` | the innermost node under the activation |
| `dom.event.current-target.form.elements.<name>/value` | the named input inside the enclosing `<form>` subtree |
| `dom.event/key` | the pressed key |
| `dom.event/type` | `click`, `key`, `change`, `submit` |
| `dom.event.detail/*` | payload of a widget-raised event |
| `dom.event.do/prevent-default` | no-op |
| — | `tui.event/row`, `tui.event/column`, `tui.event/modifiers` |

Forking (b) forks every command and rule, doubling the application for no
semantic gain. (c) is the honest naming but is a migration across the
whole standard library — take it later if the `dom` misnomer becomes a
real teaching problem.

**Two consequences to state loudly:**

- The existing "one command, one shape" hazard sharpens with two hosts. A
  browser click command and a terminal activation command reading the same
  attributes are *the same shape* and fire the same rules. Arguably the
  correct semantics — the rule expresses host-independent intent — but the
  analyzer's subset-overlap check now spans hosts.
- `dom.event.do/prevent-default` becoming a no-op keeps its nastiest edge:
  the **prevent-default trap** (an action field makes a command rule-proof,
  because the field stores no value and a rule premise over it matches
  zero rows) still applies in the TUI even though nothing is prevented.
  **The TUI host should warn** when a command it fires declares a
  `dom.event.do/*` field.

### 5.4 Affordance discovery: the keybar

Showcase component `11 · keybar` is the CLI's affordance surface: fixed
cells of `plate`/`frost` chips (`↵ eval`, `tab branch`, `g guide`,
`q quit`). A `tui` view needs the same, and there is a clean tie to the
command layer:

**Proposal — an element carrying `onkey=<command> key=g label=guide`
contributes a keybar chip automatically.** The keybar is then generated
from the bindings rather than hand-maintained beside them, and cannot drift
from what the view actually handles. Law 4 (fixed cells) means chips are
laid out at `Constraint::Length` and do not reflow while visible.

## 6. Element vocabulary, layout, and paint

### 6.1 Syntax

Keep HTML syntax. Non-negotiable: it buys the parser, the collector, the
planner, `{field}`/`{this}`, and nested `<tonk-display>` for free.

### 6.2 Tag set

Recommend a **distinct terminal vocabulary** with unknown tags degrading to
"block containing children" rather than erroring — and explicitly *not*
chasing "one template serves both facets."

The temptation to reuse `<div>`/`<span>` is template reuse, and it is a
trap: terminal layout is a fixed cell grid with no reflow, no free
overflow-scroll, and no inline-wrapping subtleties. A template satisfying
both facets will be bad at both. The `tui` facet exists precisely so they
can differ. Reuse the *syntax* and the *pipeline*, not the *template*.

Two HTML names must be kept because the pipeline depends on them:

- **`<tonk-display>`** — the composition and cross-concept-join primitive.
  `<tonk-display entity={author} model=person view=label>` must work
  identically, so `label` facets are shared between hosts (they are just
  text — a fine thing to share).
- **`<tonk-fallback>`** — the empty-state affordance, keyed on the host's
  `data-state`.

Provisional set, with showcase pane numbers where one transfers:

| Group | Elements |
| --- | --- |
| Layout | `<row>`, `<column>`, `<box>` (`┌─ title ─┐`, pane `09`), `<spacer>`, `<scroll>` |
| Text | `<text ink=…>`, `<p>` (wrapping) |
| Collections | `<list>`, `<table>` (bold-dim header + `───` rule, pane `08`) |
| Blocks | `<block>` (`▌` gutter marker, pane `09`), `<log>` line with `[ok]`/`[··]`/`[--]`/plate `!!` status (pane `04`) |
| Status | `<sigil>` (`●◐○`, pane `05`), `<spinner>` ("the run", pane `07`), `<progress>` (one stripe, pane `07`) |
| Input | `<input>`, `<textarea>`, `<checkbox>`, `<select>`, `<form>`, `<prompt>` (pane `06`) |
| Chrome | `<keybar>`/`<key>` (pane `11`), `<tabs>` |
| Tonk | `<tonk-display>`, `<tonk-fallback>` |

### 6.3 Style, layout, and the clock

**Style is one attribute.** `ink=bold|plain|dim|frost|plate`. That is the
entire style surface (§2.3). No `fg`, no `bg`, no `focus-*`. `<style>`
blocks are ignored — the collector already skips them as raw-text
elements, so this costs nothing, and a CSS subset would need a selector
engine and a cascade for a surface whose styling need is five values.

**Layout: expose ratatui's constraint vocabulary directly.**
`Constraint::{Length, Percentage, Min, Max, Fill}` is already the right
model; do not invent one. Law 4 means chips and cells are `Length`.

```
<column gap=1 pad=1>
  <row height=1><text ink=bold>{title}</text><spacer/><text ink=dim>{count}</text></row>
  <scroll grow=1>
    <list subject={this}>
      <row><text>{title}</text></row>
    </list>
  </scroll>
  <keybar/>
</column>
```

**The clock is a first-class renderer concern.** Law 2 and §2.4 mean the
frame loop is driven by `max(subscription events, animation tick)` — a
2.4 s calm cycle with 300 ms spinner sub-frames and a 1.05 s cursor blink,
repainting at ≤ 10 fps. An `alert` or `pulse` attribute opts an element
into the wash; hover/interaction calms it. **This is the requirement most
easily missed when scoping "just render the tree."**

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

### 7.1 The one refactor this plan requires

`orchestrate::render` returns a `String` and expands nested
`<tonk-display>` by re-rendering *into the HTML string*; `render_portal`
emits an `<iframe srcdoc>`. Both are HTML-specific.

Split orchestration so the shared half returns the resolved `Vec<Node>`
and does nested expansion **on the tree**, with serialization and portal
handling as an HTML-only tail. `tonk render` keeps its exact current
output; the TUI consumes the tree. This is a real refactor with real
regression risk — nested expansion is where the recursion guard and the
visited-set cycle detection live — and is **the largest piece of work in
the "free" column that is not actually free.**

Portals (`type: text/html`) have no terminal meaning. Render a placeholder
box naming the portal, not an error.

## 8. The default `tui` facet, and who authors these

If the terminal is a peer surface for every space, somebody must author a
`tui` facet for every model — or the feature is dead on arrival for every
existing space.

The browser solves this with `view!: { this: tonk:_, show: { directory: … } }`
— a wildcard-model fallback rendering any model's instances in a carousel
of nested single-entity displays. **The TUI needs the same**: a `tonk:_`
`tui` facet — pane `08`'s table is the obvious shape, one row per
instance, each cell a nested `<tonk-display view=label>` — plus the
notation fallback for single entities.

That gives every space a usable terminal view with zero authoring and
makes a hand-written `tui` facet an upgrade rather than a prerequisite.
**This is the difference between a demo and a feature** and belongs in the
first interactive milestone, not last.

## 9. Where the code lives

- **`rust/tonk-ink`** (new, native-only): the stripes primitives — the
  five ink treatments, glyph set, spinner frames, calm-cycle clock, and
  the degradation matrix (§2.5). Sourced from `tui/tokens.json` (§2.6).
  **Shared by the TUI renderer and the CLI's own output**, so `tonk eval`
  and a `tui` view cannot drift. No `ratatui` dependency.
- **`rust/tonk-tui`** (new, native-only): element vocabulary, layout,
  paint, focus model, terminal-event → transient extraction. Depends on
  `tonk-template`, `tonk-render` (parse/tree/collect/`render_nodes`),
  `tonk-schema`, `tonk-ink`, `ratatui`, `crossterm`.
- **`rust/tonk-render`**: the orchestration split from §7.1. No new deps.
- **`rust/tonk-cli`**: `tonk tui [route]` in `src/tui.rs`, implementing
  `SubscribeBackend` over `TonkSite`'s reactor (mirroring the existing
  `QueryBackend for TonkSite` impl in `src/render.rs`). Separately, and
  independently schedulable: `output.rs` adopts `tonk-ink`.

Do **not** pre-emptively split `tonk-render` into a tree crate and an HTML
crate. `tonk-tui` depending on `tonk-render` and never calling
`serialize_nodes` is fine until it isn't.

`ratatui` is already a workspace dependency in the sibling `dialog-db`
workspace (`dialog-diagnose`), so the stack has precedent.

## 10. Ratatui vs. an ink-style reconciler — and how much of ratatui

Ink's value proposition is a reconciler over a JS component tree. Tonk has
no component tree in the host: templates are branch data, plans rebuild
per frame, retained state lives in the reactor. **A reconciler solves a
problem this architecture does not have.**

**Recommend immediate-mode**: rebuild the widget tree from the resolved
`Vec<Node>` each frame. Data arrives at subscription cadence, and the
browser renderer's incremental DOM diffing exists because the DOM is
expensive to rebuild — a cell buffer is not.

**But take less of ratatui than the default.** `ratatui::widgets`
(`Gauge`, `Table`, `Block`, `Tabs`) carry their own aesthetics — colored
gauges, optional rounded borders, its own emphasis conventions — all of
which stripes contradicts (§2.2). Fighting a widget library's defaults to
reach a monochrome fixed-cell design is more work than painting it.

Use ratatui as a **layout solver and cell buffer** (`Layout`,
`Constraint`, `Buffer`, `Rect`, the crossterm backend) and paint the
stripes primitives in `tonk-ink` directly. Adopt individual `widgets` only
where one happens to already match — `Paragraph`'s wrapping is the likely
candidate.

The cost of immediate mode is §5.2: no per-widget instance state. Real,
and the reason §5.2 needs an answer before §6 gets interesting.

## 11. Milestones

- **M0 — static frame.** `tonk tui <route>` resolves a route, renders one
  frame, exits on `q`. Vocabulary: `<row>`, `<column>`, `<box>`, `<text>`,
  `<list>`, `<table>`, `<spacer>`. Ink treatments from `tonk-ink`. No
  events, no focus, no clock. Proves parse → plan → `render_nodes` →
  cell buffer end to end.
- **M1 — live.** `SubscribeBackend` over the reactor; redraw on frame
  change. Empty/loading/error states mapped onto the existing `State`
  enum. The `tonk:_` `tui` fallback from §8.
- **M2 — activation.** Focus ring (frost), tab traversal, `Enter`/`Space`
  → `onclick` → transient → transact, generated keybar (§5.4). Browser
  rules start firing from a terminal.
- **M3 — motion + input.** The calm-cycle clock, `<spinner>`,
  `<progress>`, alert pulse, cursor blink. `<input>`, `<textarea>`,
  `<checkbox>`, `<select>`, `<form>`; `onchange`/`onsubmit`; host-side
  widget state per §5.2(a).
- **M4 — chrome + composition.** `<scroll>`, `<tabs>`, mouse, nested
  `<tonk-display>`, `<tonk-fallback>`, full degradation matrix.

`tonk-ink` + `output.rs` adoption is independent of all of these and can
land first — it makes the CLI look like the design system regardless of
whether the view renderer ships.

## 12. Testing

The repo already treats browser/headless parity as first-class
(`plan/view-anchor-render-parity.md`). A third renderer makes that
three-way, but only for the shared half — the planner.

- **Plan parity**: assert `tonk-tui` and `tonk-render` produce identical
  `BindingPlan`s for the same template. Free, since they share the
  collector.
- **Paint snapshots**: ratatui's `TestBackend` renders to a fixed-size cell
  buffer; snapshot it. The TUI analogue of the existing HTML string
  assertions.
- **Degradation snapshots**: the §2.5 matrix is unusually testable —
  the same view under `NO_COLOR` / no-dim / non-UTF-8 / not-a-tty /
  < 80 cols is five snapshots against one template. Worth wiring in M0,
  before there is much to regress.
- **Token parity**: assert `tonk-ink`'s tables match `tui/tokens.json`, so
  a design-system revision fails a test rather than drifting silently.
- **Command parity**: assert the transient a terminal activation posts is
  byte-identical to the one a browser click posts for the same command
  descriptor and the same `data-*`. This is the claim §5.3 rests on; test
  it rather than assume it.

## 13. Open questions

1. **`tui/tonk-tui.md`, `tui/README.md`, `tui/tokens.json`** have not been
   read — only `showcase.html`. The spec and the rationale (which records
   *rejected* directions and a checklist for new decisions) should be read
   before §6 is treated as settled, and `tokens.json` before `tonk-ink` is
   written.
2. **Is the terminal a peer surface for every space, or a TUI-first
   authoring/inspection tool?** §8's answer changes completely. If a peer
   surface, the `tonk:_` fallback is load-bearing and M1-critical. If an
   inspection tool, a small hand-authored set of `tui` facets on the
   standard library models may be enough and §8 shrinks to nothing.
3. **Does `stripes` intend to cover data views at all, or only tool
   chrome?** The showcase's provenance line scopes it to
   "init · eval · schema · guide". A `tui` view painting user data is
   arguably outside its remit, and §6.2's vocabulary is an extension of the
   system rather than an application of it. Worth confirming with whoever
   owns `tui/README.md`'s decision checklist before extending it.
4. **Does `tui` want sub-facets?** A terminal has modes a browser does not
   — a compact status line vs. a full pane. `show: { tui: …, tui-line: … }`
   is free in the schema; whether it is a good idea is a question about how
   many facets an author can hold in their head.
5. **Does the analyzer need to know about `tui`?** It validates
   `on<event>` targets against transient descriptors today. If TUI-only
   events (`onkey`) exist it needs the vocabulary — or it needs to stop
   caring which events are legal.
