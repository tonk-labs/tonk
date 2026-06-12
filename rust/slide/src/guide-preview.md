# Previewing views

`slide preview` renders a candidate `<tonk-display>` template with the
**real** browser renderer against **live branch data**, and hands you
back the rendered HTML plus diagnostics — before you commit the template
as a `view!:`. Use it to iterate on declarative templates instead of
guessing what the `{field}` interpolation will produce.

## Why

A declarative template's failure modes are invisible until it renders:
a typo'd `{field}` renders blank, a single-occurrence bare text node can
drop out, an absent entity falls back to chrome. Preview surfaces those
explicitly so you don't ship a silently-broken view.

## The loop

Preview is a daemon plus a browser page. Set it up once, then render as
many candidate templates as you like.

1. Start the daemon (long-lived; leave it running):

   ```
   slide preview serve
   ```

   It prints a localhost URL.

2. **Ask the human to open that URL in a browser** and leave the tab
   open. The page connects back to the daemon and becomes the renderer.
   You cannot open it yourself — say so explicitly. Until a page is
   connected, `render` returns a "no preview page connected" error.

3. Render a candidate template against an entity's live data:

   ```
   echo '<article><h1>{title}</h1></article>' \
     | slide preview render --model task --this t1
   ```

   `--model` is the concept (name or URI); `--this` is the subject
   entity (name or URI) whose live fields the template renders against —
   the same data `<tonk-display>` would subscribe to. The template comes
   from stdin, or `--template <file>`.

   stdout is the rendered HTML; warnings (diagnostics) go to stderr. The
   render also appears in the open browser page, so a human can watch the
   artifact while you iterate.

## Diagnostics

`render` names the footguns instead of letting them render blank:

- `unbound-field` — `{field}` isn't on the model (with a did-you-mean).
- `empty-resolve` — the field exists but resolved empty on every row.
- `empty-frame` — the entity projected zero rows; fallback chrome
  rendered, not data.
- `value-missing-from-output` — a resolved value didn't appear in the
  HTML (the single-occurrence / iteration-anchoring trap).

Pass `--json` for one machine-readable object (`html`, `row_count`,
`diagnostics`) instead of human-readable text.

## Committing the view

Preview is a dry run — it never writes. Once the template looks right,
publish it the normal way (`view!:`, see `slide guide views`).
