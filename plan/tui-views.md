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
      <row spacing=1 width=fill>
        <text weight=bold>{title}</text>
        <text width=fill fg=muted align=right>{status}</text>
      </row>
```

Three things shape the design:

- The **notation half needs no design at all** — `tui` is already a legal
  facet (§1.1), and `alice@todo!tui` already parses.
- The **pipeline is already DOM-free** down to a `Vec<Node>` seam (§1.4).
- The **layout and style algebra is the interesting design problem** (§6),
  and the reference points — elm-ui's vocabulary, ink's use of a real
  flexbox engine — point at a specific answer.

The remaining risk is concentrated in the interaction model (§5).

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

## 2. Reference points and what each one contributes

### 2.1 elm-ui — the vocabulary

`mdgriffith/elm-ui` is the primary reference for the *authoring surface*.
Its thesis is that CSS layout is too large and too ambiguous, and that a
small total vocabulary is better. What it actually provides (verified
against `src/Element.elm` on `master`):

- **Primitives**: `none`, `text`, `el` (single child), `row`, `column`,
  `wrappedRow`, `paragraph`, `textColumn`, `table` / `indexedTable`.
- **`Length`**: `px Int`, `shrink` (content-sized), `fill` (= `Fill 1`),
  `fillPortion Int`, with `|> minimum n` / `|> maximum n` modifiers.
  `el`, `row` and `column` all **default to `width shrink, height shrink`**.
- **No margin, at all.** Only `padding` (outer edge → content:
  `padding`/`paddingXY`/`paddingEach`) and `spacing` (between children:
  `spacing`/`spacingXY`/`spaceEvenly`). The doc is explicit: *"There's no
  concept of margin in elm-ui, instead we have padding and spacing."* This
  kills margin collapse as a category of bug. On a `paragraph`, `spacing`
  sets line spacing.
- **Alignment is declared on the child, interpreted by the parent**:
  `centerX`, `centerY`, `alignLeft/Right/Top/Bottom`. In a `row`, an
  aligned child *pushes* the others. `row` defaults to children centered
  on the cross axis; `column` defaults to top-left.
- **Nearby elements**: `above`, `below`, `onRight`, `onLeft`, `inFront`,
  `behindContent` — *"put this element below this other element, but don't
  affect the layout when you do."* Overlays and dropdowns without absolute
  positioning in the author's face.
- **State-scoped styling**: `focused`, `mouseOver`, `mouseDown` as
  `Decoration` attributes. No selector engine, no cascade — a state-keyed
  attribute bundle on the element itself.
- **`Element.Input` forces a `Label`** on every input (`labelAbove`,
  `labelLeft`, …, and `labelHidden` still requires the text). Accessibility
  is not optional.
- **`explain`** draws debug borders around every element.
- Style modules are orthogonal to layout: `Font` (color, size, weight
  ladder, alignment, underline/strike/italic), `Background` (color,
  gradient), `Border` (color, width, style, rounded).

Nearly all of this maps onto a terminal better than it maps onto a
browser, because a cell grid has no reflow subtleties, no margin
collapse, and no baseline alignment to get wrong.

### 2.2 ink / yoga — the engine, not the API

The interesting part of ink is that it does not hand-roll layout: it
delegates to yoga, a real flexbox implementation, and supplies a
measure function for text. That is the right shape and the wrong API —
ink exposes raw flexbox props (`flexGrow`, `justifyContent`), which is
exactly the ambiguity elm-ui exists to remove.

**Take the architecture from ink, the vocabulary from elm-ui**: elm-ui's
surface compiles down to flexbox anyway. §6.3 picks the Rust engine.

### 2.3 `stripes` — a theme, not a law

`tonk-labs/gooey@mvp:tui/showcase.html` (v0.2) is a CLI *output* design
system: its components are numbered against `tui/demo.sh` and are the
surfaces of a non-interactive command (log lines, sync glyphs, prompts,
progress, tables, blocks, help, keybar), scoped in its own provenance line
to "init · eval · schema · guide".

**It is a direction of interest, not a constraint on the renderer.** Its
first law — "ink only, no color codes exist" — is a *theme* decision. A
color-capable renderer can express a colorless theme; a colorless renderer
can express nothing else. So: **build the color-capable system, ship
stripes as the default theme** (§6.7).

What stripes contributes regardless of palette, because these are
renderer requirements rather than aesthetic ones:

- **A motion budget**, which implies an animation clock independent of
  data change: 2.4 s calm cycle, 8 × 300 ms spinner frames, ≤ 10 fps
  progress repaint, 1.05 s cursor blink, "> 400 ms gets a spinner, > 3 s
  gets a progress bar". §6.9.
- **A degradation matrix**: `NO_COLOR`; no-dim → plain; non-UTF-8
  (`▀`→`#`, `●◐○`→`*o.`, box→`+-|`); not-a-tty → no SGR at all; < 80 cols
  → reduced chrome. §6.8. This generalizes into the capability ladder that
  color needs anyway.
- **Fixed cells** — chips never move or resize while visible. A real
  layout constraint (`px`, never `fill`, for chip-like elements).
- **`tui/tokens.json`** — machine-readable ink treatments, glyphs, spinner
  frames, logo bitmaps. Consume it; do not transcribe it (§6.7).

`tui/tonk-tui.md` (spec) and `tui/README.md` (rationale, including
*rejected* directions and a decision checklist) have not been read and
should be before the theme layer is written.

## 3. What transfers for free

| Piece | Status |
| --- | --- |
| `view` concept / `show` dictionary | `tui` is a legal facet today |
| Route grammar (`entity@model!tui`) | works today |
| `tonk-template` planner, `resolve`, `fold` | DOM-free by construction |
| `tonk-render::{parse, tree, collect, render_nodes}` | tag-name-agnostic; only `is_void_tag` / `is_raw_text_element` are HTML-specific |
| Reactor `branch.subscribe(ConceptQuery)` | native, no service worker |
| Commands, transients, rules | host-independent; the reactor never sees the event |

The `html5gum` tokenizer parses `<row spacing=1><text weight=bold>{title}</text></row>`
today. The *authoring* story — write a template, get a plan, get a
resolved tree — needs zero new code.

## 4. What has to be built

1. A **layout and style algebra**, and an engine under it (§6)
2. An **interaction model** with no pointer and no focus manager (§5)
3. **Terminal event → transient extraction** (§5.3)
4. A **subscription seam** — `QueryBackend` is one-shot (§7)
5. A **default `tui` facet** — the `tonk:_` analogue (§8)

## 5. The interaction model

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
- **Focus styling** follows elm-ui's `focused` decoration: state-prefixed
  attributes on the element itself (`focused-bg=`, `focused-weight=`), no
  selector engine (§6.6). `mouseOver`/`mouseDown` become `hover-*` (mouse
  terminals only) and `active-*` (armed).
- **Mouse**: crossterm gives real clicks; a click also fires `onclick`,
  with `tui.event/row` / `tui.event/column` available.
- **Scrolling**: elm-ui has `scrollbars`/`scrollbarX`/`scrollbarY` and
  `clip`. Same attributes; the container owns an offset and consumes
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

### 5.4 Labels, and affordance discovery

Steal elm-ui's `Element.Input` rule: **every input requires a label**, and
hiding it still requires supplying the text. In a terminal a label is not
only accessibility — it is what a generated keybar, a `--json` dump, and a
non-tty render have to print.

Extend it to bindings: an element carrying `onkey=<command> key=g
label=guide` **contributes a keybar chip automatically**. The keybar is
then generated from the bindings rather than maintained beside them, and
cannot drift from what the view actually handles. Stripes' "fixed cells"
law applies: chips are `px`, never `fill`, so they do not reflow while
visible.

## 6. Layout and style: an elm-ui algebra over a flexbox engine

This is the centre of the design.

### 6.1 The shape of the answer

**elm-ui for the authoring surface; a real flexbox engine underneath.**
elm-ui's own implementation compiles to CSS flexbox, so this is not a
compromise between the two references — it is what elm-ui already is, and
what ink already does with yoga, with ink's raw-flexbox API replaced by
elm-ui's smaller one.

### 6.2 The vocabulary, as template attributes

| elm-ui | template |
| --- | --- |
| `el`, `row`, `column`, `wrappedRow` | `<el>`, `<row>`, `<column>`, `<wrapped-row>` |
| `paragraph`, `textColumn`, `text`, `none` | `<paragraph>`, `<text-column>`, text nodes, absent |
| `table`, `indexedTable` | `<table>` with the `{this}` repeat root as the row |
| `width fill` / `px 20` / `shrink` / `fillPortion 2` | `width=fill` / `width=20` / `width=shrink` / `width=fill:2` |
| `|> minimum n`, `|> maximum n` | `min-width=`, `max-width=` |
| `padding`, `paddingXY`, `paddingEach` | `pad=`, `pad-x=`/`pad-y=`, `pad-top=`… |
| `spacing`, `spacingXY`, `spaceEvenly` | `spacing=`, `spacing-x=`/`spacing-y=`, `space-evenly` |
| `centerX`, `alignRight`, … | `align=center-x`, `align=right`, … (on the child) |
| `above`, `below`, `onRight`, `inFront`, `behindContent` | `<above>`, `<below>`, `<on-right>`, `<in-front>`, `<behind>` as marked children |
| `clip`, `scrollbars`, `scrollbarY` | `clip`, `scroll`, `scroll-y` |
| `focused`, `mouseOver`, `mouseDown` | `focused-*`, `hover-*`, `active-*` prefixes (§6.6) |
| `Font.color`, `Background.color`, `Border.*` | `fg=`, `bg=`, `border=`, `border-x=`… (§6.5) |
| `explain` | `tonk tui --explain` (§6.10) |

Keep elm-ui's defaults: `el`/`row`/`column` default to `shrink` on both
axes; `row` centres children on the cross axis; `column` is top-left.
Keep the **no-margin rule** verbatim — `pad` and `spacing` only.

Two HTML names must survive because the pipeline depends on them:
**`<tonk-display>`** (composition and cross-concept joins — so `label`
facets are shared between hosts) and **`<tonk-fallback>`** (empty state,
keyed on the host's `data-state`).

Unknown tags degrade to `<el>`, not an error.

### 6.3 The engine: three options

- **(a) `taffy`** (`0.14.0`, updated 2026-08; used by Zed, Dioxus, Bevy).
  Full flexbox + grid, `Position::Absolute`, min/max, flex-wrap, and a
  **leaf measure-function hook** — the seam ink uses yoga for.
- **(b) ratatui's own `Layout`.** Already integer and cell-native, and
  closer to elm-ui than it first appears: `Constraint::Length` = `px`,
  `Fill(n)` = `fillPortion n`, `Min`/`Max` = the modifiers,
  `Layout::spacing` = `spacing`, `Flex::{Start,Center,SpaceBetween,…}` =
  alignment.
- **(c) hand-rolled.**

**Recommend (a).** Ratatui's `Layout` solves one axis of one split and
does **no content-based sizing** — but `shrink` is elm-ui's *default* on
`el`/`row`/`column`, so content sizing is the base case, not an edge case.
Ratatui also gives no flex-wrap (`wrappedRow`), no absolute positioning
(`inFront`/`behindContent`), and no width-dependent height (`paragraph`).
taffy gives all four. (c) is re-deriving taffy badly.

The cost of (a) is a translation layer from elm-ui semantics to taffy
style structs, and it is a real one — elm-ui's alignment-pushes-siblings
behaviour in a `row` is not a plain `align-self`.

### 6.4 Where a terminal diverges from both references

These are the parts neither elm-ui nor ink can be copied on.

- **Cells are integers.** taffy computes in `f32` and has a rounding pass
  that rounds **cumulative absolute positions**, not individual sizes,
  precisely so adjacent boxes never gap or overlap by one unit. Treat one
  cell as one "pixel" and keep that pass on. Rounding must also be
  *stable* across frames, or a `fill:1 / fill:1 / fill:1` split of 80
  columns will shimmer between 26/27/27 and 27/26/27 on unrelated
  redraws.
- **Cells are not square.** A terminal cell is roughly 1 : 2. elm-ui's
  uniform `padding 10` is simply wrong here: `pad=1` is a much larger
  vertical step than horizontal. Recommend making `pad-x` / `pad-y`
  (elm-ui's `paddingXY`) the *idiomatic* form and either defaulting `pad=n`
  to a 2 : 1 x : y ratio or refusing the uniform form outright. This is a
  small decision with a large effect on whether authored layouts look
  deliberate.
- **Text measurement is the hard part, and it is ours.** taffy delegates
  leaf measurement to us. Terminal width is not character count: it is
  grapheme clusters scored by `unicode-width` — East Asian wide characters
  are 2 cells, combining marks 0, and emoji/ZWJ sequences need cluster
  segmentation first. `unicode-width` is **already a tonk workspace
  dependency** (`tonk-cli/src/listing.rs`), so half the problem is
  acknowledged; `unicode-segmentation` is the missing half. A naive
  `str::len()` or `chars().count()` measurer will be wrong for real data
  and the bug will look like a layout bug.
- **Wrapping makes height depend on width.** `<paragraph>` needs
  line-breaking (`unicode-linebreak`, or `textwrap` which already handles
  the width scoring) inside the measure function. taffy's measure hook
  receives available space precisely for this; get it right once, in the
  measurer, rather than in every widget.
- **Borders cost a whole cell.** `Border.width 1` in a browser is
  sub-character; in a terminal it consumes a full row/column. Border width
  is 0 or 1 and participates in layout as padding.

### 6.5 Color and emphasis are two orthogonal axes

Keep them separate, because SGR keeps them separate.

- **Emphasis** (`weight=bold|normal|dim`, `underline`, `strike`,
  `reverse`) are SGR attributes, present on every terminal, independent of
  any palette.
- **Color** (`fg=`, `bg=`, `border-color=`) is capability-tiered.

A **capability ladder** with an explicit downgrade at each rung, exactly
parallel to the glyph degradation of §6.8:

```
truecolor (24-bit)  →  256 indexed  →  16 ANSI (terminal theme)  →  none
```

Templates should be able to say color two ways:

1. **Semantic tokens** — `fg=muted`, `bg=surface`, `fg=danger` — resolved
   through the active theme (§6.7). Preferred, because the terminal's own
   theme has opinions and a token can carry a hand-picked value for *each*
   rung rather than a nearest-neighbour approximation.
2. **Literals** — `fg=#8a7f6d` — the escape hatch, downgraded by
   nearest-neighbour when the terminal cannot do truecolor.

Detection from `COLORTERM` / `TERM` / terminfo, overridable by flag, with
`NO_COLOR` forcing the bottom rung. Ratatui's `Color` enum already spans
`Rgb`, `Indexed` and the 16 named, so the whole ladder is expressible in
the backend.

**Design point worth arguing about:** at the 16-ANSI rung the terminal's
own theme supplies the actual colors, so a token like `danger` renders as
whatever the user's theme calls red. That is usually *better* than a
literal, and it is the reason to push authors toward tokens rather than
hex.

### 6.6 State-scoped styling without a selector engine

elm-ui's `focused` / `mouseOver` / `mouseDown` are attribute bundles on
the element, not selectors. Adopt that directly as attribute prefixes:

```html
<el pad-x=1 bg=surface focused-bg=accent focused-fg=on-accent onclick=open data-todo={this}>
  {title}
</el>
```

No cascade, no specificity, no `<style>` block (the collector already
skips `<style>` as a raw-text element, so ignoring it costs nothing). The
state set is small and closed: `focused-`, `hover-`, `active-`, and
`disabled-`.

### 6.7 Themes, and `tokens.json`

A **theme** resolves semantic tokens to per-rung values, and supplies the
glyph set and motion timings. Ship at least two:

- **`stripes`** (default): tokens resolve to no color at all — emphasis
  only (bold / plain / dim / reverse) — which is exactly §2.3's "ink only"
  expressed as a theme rather than enforced as a law. Glyphs, spinner
  frames and motion timings come from `tui/tokens.json`; **vendor that
  file or mirror it with a parity test**, do not transcribe it.
- **`terminal`**: tokens resolve to the 16 ANSI names, deferring entirely
  to the user's own terminal theme.

A space could eventually assert its own theme as branch data, which is the
natural tonk-shaped answer, but that is not needed to ship.

### 6.8 Capability degradation

One matrix covering color, glyphs and motion:

| Condition | Behaviour |
| --- | --- |
| `NO_COLOR` | bottom color rung; emphasis only |
| no truecolor | tokens take their 256 (then 16) value; literals nearest-neighbour |
| no dim | `dim` → plain; washed chips → `[bracketed]` |
| non-UTF-8 | `▀`→`#`, tracks `.`; `●◐○`→`*o.`; box drawing→`+-\|` |
| not a tty | no SGR, no spinner, no cursor writes; plain line output |
| < 80 cols | reduced chrome (lockup instead of banner) |

This is a **renderer responsibility, not an author responsibility**, and
it is unusually testable (§12).

### 6.9 The clock

The motion budget of §2.3 means the frame loop is driven by
`max(subscription events, animation tick)` — a 2.4 s calm cycle with
300 ms spinner sub-frames and a 1.05 s cursor blink, repainting at
≤ 10 fps, with `> 400 ms` work getting a spinner and `> 3 s` a progress
bar. **This is the requirement most easily lost when scoping the work as
"just render the tree".**

### 6.10 `explain`

elm-ui's `explain` draws a debug border around every element. A template
language with no devtools needs this more than elm-ui does: `tonk tui
--explain` should outline every box and label it with its resolved
`Length`s. Cheap, and the difference between debuggable and not.

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
`tui` facet — a `<table>` with one row per instance, each cell a nested
`<tonk-display view=label>` — plus the notation fallback for single
entities.

That gives every space a usable terminal view with zero authoring and
makes a hand-written `tui` facet an upgrade rather than a prerequisite.
**This is the difference between a demo and a feature** and belongs in the
first interactive milestone, not last.

## 9. Where the code lives

- **`rust/tonk-layout`** (new, native-only): the elm-ui algebra over
  taffy — attribute parsing to `Length`/`pad`/`spacing`/alignment, the
  terminal text measurer (grapheme clusters × `unicode-width`, with
  line-breaking for `<paragraph>`), and the integer-cell rounding
  discipline. Depends on `taffy`, `unicode-width`,
  `unicode-segmentation`. **No `ratatui`, no `tonk-*`** — this is a
  standalone, heavily unit-testable crate, and keeping it free of both
  is what makes §12's layout tests cheap.
- **`rust/tonk-theme`** (new, native-only): semantic tokens, the color
  capability ladder, glyph sets, motion timings; `stripes` and `terminal`
  themes; sourced from `tui/tokens.json`. **Shared with `tonk-cli`'s own
  output**, which today emits plain text with no styling crate in the
  workspace at all — so `tonk eval` and a `tui` view cannot drift.
- **`rust/tonk-tui`** (new, native-only): element vocabulary, paint, focus
  model, the clock, terminal-event → transient extraction. Depends on
  `tonk-layout`, `tonk-theme`, `tonk-template`, `tonk-render`,
  `tonk-schema`, `ratatui`, `crossterm`.
- **`rust/tonk-render`**: the orchestration split from §7.1. No new deps.
- **`rust/tonk-cli`**: `tonk tui [route]` in `src/tui.rs`, implementing
  `SubscribeBackend` over `TonkSite`'s reactor (mirroring the existing
  `QueryBackend for TonkSite` impl in `src/render.rs`). Separately and
  independently schedulable: `output.rs` adopts `tonk-theme`.

Do **not** pre-emptively split `tonk-render` into a tree crate and an HTML
crate. `tonk-tui` depending on `tonk-render` and never calling
`serialize_nodes` is fine until it isn't.

`ratatui 0.29` is already a workspace dependency in the sibling
`dialog-db` workspace (`dialog-diagnose`), so the stack has precedent.

## 10. Immediate mode, and how much of ratatui to take

Ink's reconciler exists because ink owns a JS component tree. Tonk has no
component tree in the host: templates are branch data, plans rebuild per
frame, retained state lives in the reactor. **A reconciler solves a
problem this architecture does not have** — which is why §2.2 takes ink's
*engine* choice and not its runtime.

**Recommend immediate mode**: rebuild the layout tree from the resolved
`Vec<Node>` each frame. Data arrives at subscription cadence, and the
browser renderer's incremental DOM diffing exists because the DOM is
expensive to rebuild — a taffy tree over a cell buffer is not. If
profiling later says otherwise, taffy supports partial relayout via dirty
marking, so the escape hatch exists.

**Take ratatui as backend and buffer, not as a widget set.** With
`tonk-layout` owning geometry, ratatui's role is `Buffer`, `Rect`, `Span`
styling and the crossterm backend. Its `Layout` is superseded by §6.3(a),
and its `widgets` carry their own aesthetics and their own layout
assumptions. Adopt individual widgets only where one already matches —
`Paragraph`'s wrapping is the likely candidate, and only if its width
scoring agrees with our measurer.

The cost of immediate mode is §5.2: no per-widget instance state. Real,
and the reason §5.2 needs an answer before §6 gets interesting.

## 11. Milestones

- **M0 — geometry.** `tonk-layout` standalone: attributes → taffy →
  integer cell rects, with the terminal text measurer. No tonk
  dependencies, no rendering. This is where the design is proven or found
  wrong, and it is testable without a terminal at all.
- **M1 — static frame.** `tonk tui <route>` resolves a route and paints
  one frame; `q` exits. `<row>`, `<column>`, `<el>`, `<text>`,
  `<paragraph>`, `<table>`. `stripes` theme. `--explain`. Proves
  parse → plan → `render_nodes` → layout → cell buffer.
- **M2 — live.** `SubscribeBackend` over the reactor; redraw on frame
  change. Empty/loading/error states mapped onto the existing `State`
  enum. The `tonk:_` `tui` fallback from §8.
- **M3 — activation.** Focus ring, tab traversal, `focused-*` decorations,
  `Enter`/`Space` → `onclick` → transient → transact, generated keybar
  (§5.4). Browser rules start firing from a terminal.
- **M4 — color and motion.** The capability ladder (§6.5, §6.8), the
  `terminal` theme, the clock (§6.9), spinner and progress.
- **M5 — input and composition.** `<input>`, `<textarea>`, `<checkbox>`,
  `<select>`, `<form>` with required labels; `onchange`/`onsubmit`;
  host-side widget state per §5.2(a); `<scroll>`, nearby elements, mouse,
  nested `<tonk-display>`, `<tonk-fallback>`.

`tonk-theme` + `output.rs` adoption is independent of all of these and can
land first — it makes the CLI look like the design system whether or not
the view renderer ships.

## 12. Testing

The repo already treats browser/headless parity as first-class
(`plan/view-anchor-render-parity.md`). A third renderer makes that
three-way, but only for the shared half — the planner.

- **Layout unit tests** (`tonk-layout`, no terminal): a tree of attributes
  in, integer rects out. Cover `shrink` chains, `fill:n` splits that do
  not divide evenly, min/max clamping, wrapping, and the stability
  property — the same input must give the same rects on every frame.
- **Measurement tests**: CJK (2 cells), combining marks (0), ZWJ emoji
  sequences, and mixed runs. This is where a naive implementation breaks
  on real data.
- **Plan parity**: assert `tonk-tui` and `tonk-render` produce identical
  `BindingPlan`s for the same template. Free, since they share the
  collector.
- **Paint snapshots**: ratatui's `TestBackend` renders to a fixed-size cell
  buffer; snapshot it. The TUI analogue of the existing HTML string
  assertions.
- **Capability snapshots**: the §6.8 matrix is unusually testable — one
  template under `NO_COLOR` / no-truecolor / no-dim / non-UTF-8 / not-a-tty
  / < 80 cols is six snapshots. Worth wiring early, before there is much
  to regress.
- **Token parity**: assert `tonk-theme` matches `tui/tokens.json`, so a
  design-system revision fails a test rather than drifting silently.
- **Command parity**: assert the transient a terminal activation posts is
  byte-identical to the one a browser click posts for the same command
  descriptor and the same `data-*`. This is the claim §5.3 rests on; test
  it rather than assume it.

## 13. Open questions

1. **Is the terminal a peer surface for every space, or a TUI-first
   authoring/inspection tool?** §8's answer changes completely. If a peer
   surface, the `tonk:_` fallback is load-bearing and M2-critical. If an
   inspection tool, a small hand-authored set of `tui` facets may be
   enough and §8 shrinks to nothing.
2. **Non-square cells: does `pad=n` mean n cells, or n vertical and 2n
   horizontal?** (§6.4) A small decision with a large effect on whether
   authored layouts look deliberate. Worth deciding by building both and
   looking, not by reasoning.
3. **How far does elm-ui's alignment model survive translation to taffy?**
   In elm-ui a `row` child with `alignRight` *pushes* its siblings, which
   is not plain `align-self`. Either it lowers to a `justify-content`
   variant per aligned-child pattern, or the model is simplified. This is
   the most likely place the elm-ui-over-flexbox mapping leaks.
4. **`tui/tonk-tui.md` and `tui/README.md`** have not been read — only
   `showcase.html`. The README records *rejected* directions and a
   decision checklist; it is the document that would say whether a
   color-capable renderer contradicts a decision already made
   deliberately.
5. **Does `tui` want sub-facets?** A terminal has modes a browser does not
   — a compact status line vs. a full pane. `show: { tui: …, tui-line: … }`
   is free in the schema; whether it is a good idea is a question about how
   many facets an author can hold in their head.
6. **Does the analyzer need to know about `tui`?** It validates
   `on<event>` targets against transient descriptors today. If TUI-only
   events (`onkey`) exist it needs the vocabulary — or it needs to stop
   caring which events are legal.
