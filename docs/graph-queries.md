# Typed graph reads

A concept field can name another concept rather than a primitive type:

```yaml
concept!: &person
  with:
    name: { the: example.person/name, as: text, description: "Name" }
  maybe:
    manager: { the: example.person/manager, as: person, description: "Manager" }
```

Targets are bookmarks. Self-reference and mutually recursive declarations do
not recursively hash descriptors. Existing primitive-only concepts retain their
identity and encoding. Typed declarations derive their identity from the ordinary
descriptor plus the edge bookmarks; `this:` can still pin it.

`person:` through `/evaluate` returns ordinary root rows and a `graph` member
on the match block. The graph contains `roots` and normalized `nodes`: each
entity appears once, with attribute bags keyed by canonical concept entity.
Missing scalar fields are omitted; many-valued fields are vectors. Incomplete
nested entities stay visible. Applications can use Dialog's shared `check_bag`
function to explain missing required fields.

Required acyclic root edges require local target conformance. Optional and
cyclic edges shape results. Nested edges never impose recursive conformance.

Query-side equalities explicitly name their stage:

```yaml
issue:
  graph:
    matching:
      owner: { name: "Alice" }
    shaping:
      comments: { body: "Repro attached" }
```

Matching may remove an issue; shaping restricts its comments without removing
the issue. Constants reach Dialog's scans. Matching on cyclic edges is rejected.
Graph constraints currently accept constants, not variables.

For streaming, send the ordinary concept query body to the existing repository
`/query?graph=person` endpoint with `Accept: text/event-stream`. Connections get a
`graph_snapshot` containing `graph`, followed by `graph_delta` frames containing
Dialog's normalized `delta` (roots, nodes and changed fields). Reconnecting gets
a fresh snapshot. Without the header, this endpoint returns one normalized graph.
Omitting the `graph` parameter preserves the row protocol. Equal graph
subscriptions share the retained Dialog engine and existing reactor cleanup.

A subscription captures its schema on open; reconnect after changing bookmarks.
Query-side constraints are available through notation; the streaming URL selects
the declared graph without extra constraints.
