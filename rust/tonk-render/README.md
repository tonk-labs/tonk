# tonk-render

Headless (no-DOM) rendering of `tonk-display` view templates to HTML strings.

This is the server-side half of the `<tonk-display>` rendering pipeline: it
parses a `{field}`-interpolated HTML template, collects bindings, runs them
through the shared [`tonk-template`] planner, and renders against query
conclusions, all without a browser. `slide render` uses it to produce the same
markup the browser component produces, headlessly.

## Pipeline

```rust
use tonk_render::{parse_fragment, collect_bindings, render, Conclusion};
use tonk_template::{this_repeat_root, split_plan};

let mut roots = parse_fragment(template_html);       // tl -> owned Node tree
let bindings  = collect_bindings(&mut roots);        // split text nodes, collect paths
let plan      = split_plan(bindings.clone(), this_repeat_root(&bindings));
let html      = render(&roots, &plan, &conclusions); // plan + rows -> HTML string
```

- **`parse`** turns the template into an owned [`tree::Node`] whose children
  (element + text + comment) are indexed together exactly like the DOM's
  `child_nodes()`, so the planner's `Vec<usize>` paths navigate it identically.
- **`collect`** mirrors the browser collector: it splits interpolated text nodes
  in place so each `{field}` gets its own targetable node, applies the `html:`
  force-attribute rule, and skips `<style>`/`<script>` content. The emitted
  binding paths feed the shared planner, so native plan == browser plan by
  construction.
- **`render`** is one-shot (no diffing): clone the repeat element per
  conclusion, stamp `with=<this>`, apply the body and iterations, serialize.

## Browser parity

`tests/compat.rs` asserts the output matches the exact `outerHTML` a real
`<tonk-view>` produces (captured via Chrome DevTools), after normalizing the
browser-only artifacts (the `<tonk-view>` host wrapper and the
`<!--tonk-repeat-->` / `<!--tonk-iter:FIELD-->` insertion anchors).

Parity holds for well-formed, lowercase templates with literal-string fields.
Known divergences are tracked in `plan/ssr-review.md`: parser tree construction
for tables (implicit `<tbody>`) and tag-omitted markup (`<li>`/`<p>`), and
attribute-value dispatch for non-string / boolean fields. Author templates
well-formed and lowercase until those are addressed.

## Dependencies

`tl` (parsing) + `tonk-template` (the shared planner). DOM-free and
dialog-free.
