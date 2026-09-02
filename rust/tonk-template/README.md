# tonk-template

The shared, DOM-free pieces of the `<tonk-display>` rendering pipeline: the
binding **planner**, plus the **resolve** query builders and **fold** row
collapser. Used by both `tonk-display` (the browser component, which walks a
real `DocumentFragment`) and `tonk-render` (the headless renderer, which walks a
parsed `tl` tree), so both produce the same plan from the same template by
construction.

## What's here

- **planner** (`lib.rs`) — `parse_segments`, the `Binding` / `PlanNode` /
  `BindingPlan` / `RepeatPlan` types, `this_repeat_root`, `build_plan_nodes`,
  `split_plan`, and `render_segments_with_shadow`. A view template is a
  `{field}`-interpolated HTML fragment; the planner turns the *bindings*
  collected from it (text nodes and attributes containing `{...}`, each
  addressed by a child-index path) into a `BindingPlan`: the render-once
  **chrome** plus the per-conclusion **repeat** body, with cardinality-many
  fields lowered to `PlanNode::Iteration`. This half is std-only and
  target-agnostic.
- **[`resolve`]** — the wire-query builders (`phase1_query`, `name_query`,
  `view_query`, `entity_query`, `instances_query`, `view_predicate`,
  `parse_source`) that drive the model → view → entity resolution. They
  produce `tonk_schema::query::Query`.
- **[`fold`]** — `select_rows`, which groups flat query rows by `this` into one
  folded `Conclusion` per subject, collapsing cardinality-many fields to
  `Ipld::List`, and `show_template`, which picks one facet's template out of a
  folded `show` dictionary.

## Dependencies

The planner is DOM- and target-free. `resolve` and `fold` depend on
`tonk-schema`'s wire types (`Query` / `Conclusion`), which transitively pull in
the dialog stack — so the crate as a whole is not dependency-light, only DOM-
and target-free.

## Consumers

- `tonk-display` re-exports `resolve` and `fold` and adds the web-sys DOM
  snapshot + renderer.
- `tonk-render` adds the `tl` parser + native node tree + string renderer.
- `tonk` (the `render` command) drives resolve + fold + the planner directly.
