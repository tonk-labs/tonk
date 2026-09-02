# `tonk-tui-poc`

A proof of concept for `plan/tui-views.md`: render a `tui` view facet
into terminal cells.

```
cargo run -p tonk-tui-poc -- \
  --template rust/tonk-tui-poc/demo/todo.tui.html \
  --data rust/tonk-tui-poc/demo/todo.json \
  --size 56x12
```

```
  todo                                          4 open

  ┌──────────────────────────────────────────────────┐
  │ [ ] port the view pipeline                   ada │
  │ [x] measure text in cells                  grace │
  │ [ ] 日本語 のタイトル                      kenji │
  │ [ ] decide pad-x vs pad-y                    ada │
  └──────────────────────────────────────────────────┘

   ↵ open    n new    d done                   q quit
```

Flags: `--size WxH`, `--explain` (outline every element, elm-ui style),
`--tree` (print resolved rectangles instead of painting), `--plain` (no
SGR, for snapshots), `--colour truecolor|256|ansi|none`.

## What it is meant to establish

1. **The existing view pipeline needs no changes for a non-HTML
   vocabulary.** `pipeline.rs` calls `tonk_render::parse_fragment`,
   `tonk_render::collect_bindings`, `tonk_template::this_repeat_root`,
   `tonk_template::split_plan_with_scalars` and
   `tonk_render::render_nodes` — the same calls `tonk render` makes —
   and diverges only at the `Vec<Node>` seam. `<row spacing-x=1>` plans
   exactly like `<div>`, `{field}` interpolates, and the `{this}` repeat
   root clones per conclusion while the chrome renders once.
2. **The elm-ui algebra lowers onto flexbox.** `tonk-layout` is a
   standalone crate — no tonk dependency, no ratatui — turning an
   attribute tree into integer cell rectangles via `taffy`, at one CSS
   pixel per cell.
3. **elm-ui's alignment survives the translation.** An aligned child
   *pushes* its siblings, which is neither `align-self` nor any single
   `justify-content` value; grouping children by alignment and growing
   a spacer between the groups reproduces the elm-ui doc's own
   `|-|-|    |-|    |-|` example exactly. That was flagged as the most
   likely place the mapping would leak; it does not.
4. **Terminal text measurement is the load-bearing correctness
   problem**, as predicted. Two bugs found while building this were the
   same bug on opposite sides of the pipeline: measuring by `char`
   rather than grapheme cluster over-sizes CJK, and emitting a wide
   grapheme's covered cells as spaces pushes the rest of the line
   right. Both present as *layout* bugs. `tests/render.rs` pins the
   second; `tonk-layout`'s `measure` tests pin the first.

## What it deliberately does not do

- **One frame to stdout, no event loop.** No focus ring, no key
  handling, no commands, no transients. That is M2 in the plan, and it
  is where the remaining design risk lives.
- **No reactor.** Conclusions come from a JSON file, so this does not
  touch the orchestration refactor (`plan/tui-views.md` §7.1) that live
  rendering needs.
- **No `<scroll>`.** Nothing shrinks below its content — elm-ui has no
  such state — so a subtree can exceed its box. The painter clips
  (a `<box>` clips its own contents); turning that into a scrollable
  region is later work.
- **No motion.** The 2.4 s clock, spinner and progress stripe are M4.
- **`--explain` overwrites corner glyphs** rather than washing the
  background, because a terminal has no sub-cell stroke to draw an
  outline in.

## The layout crate

`tonk-layout` is the piece worth keeping regardless of what happens to
this binary, and it is deliberately engine-swappable: if the scope
question in `plan/tui-views.md` §13.1 resolves toward an inspection tool
with fixed chrome, the same public API can sit on ratatui's own
`Layout` plus a measure pass instead of `taffy`. `tests/layout.rs` is
the contract either engine has to satisfy.
