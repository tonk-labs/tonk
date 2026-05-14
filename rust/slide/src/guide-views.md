# Slide-specific notation: HTML views

slide ships two share verbs that turn local data into a URL a human can
paste into a browser: `slide share concept <name>` and `slide share view
<name|entity>`. The concept share is automatic — any concept the agent
has defined is shareable as-is. The view share targets entities that
carry a `text/html` claim, and that claim has to come from somewhere.

slide does not invent a write path for HTML; it goes through `slide eval`
like everything else. The convention is one attribute and one concept,
which you assert once per repository:

```yaml
attribute!: &html-body
  description: "HTML body of a slide-authored view"
  the:         text/html
  as:          text
  cardinality: many

concept!: &view
  description: "An HTML view, served via the host route"
  with:
    body: html-body
```

The non-obvious bit is `the: text/html`. Attribute URIs (the `the:`
field on `attribute!`) are validated at the dialog layer, which
accepts any `<domain>/<name>` shape — including MIME-style strings
like `text/html`. Once that attribute is declared, `view` is a normal
concept and you write views the way you write any other concept
assertion.

## You are writing a body fragment, not a full document

A view body is the **inside of `<body>`**, nothing more. The host
wraps it at serve time with a fixed shell that provides the
doctype, a `<meta charset>`, and the `<tonk-concept>` runtime script
that hydrates live data into your templates. Write content, not
chrome:

```yaml
view!: &my-task-list
  body: |
    <h1>Tasks</h1>
    <tonk-concept source="task">
      <ul>
        <template>
          <li>{title} — {status}</li>
        </template>
      </ul>
    </tonk-concept>
```

Do **not** write `<!doctype>`, `<html>`, `<head>`, `<body>`, `<title>`,
or `<script>` tags yourself. They will either be stripped or
double-wrapped — neither does what you want. If you need page-level
styling, put a `<style>` tag at the top of the body fragment.

The runtime that activates `<tonk-concept>` is provided by the host
and ships with every served view. You don't import it, link to it,
or include it — assume it's there.

Git-tag semantics apply: re-asserting the same body is idempotent
(same content → same content-derived entity), a different body
produces a new entity and re-points the `my-task-list` name. To
retract every attribute in the projection, query the entity into a
variable and use `..: _`:

```yaml
view:
  this: ?v
  body: ?body

view!:
  this: ?v
  ..:   _
```

To list views: `slide views`. To share one: `slide share view my-task-list`.
The listing is claim-driven — it surfaces every entity holding a
`text/html` claim regardless of which concept (if any) it was asserted
through — so a non-`view` schema that happens to assert `text/html` still
shows up.
