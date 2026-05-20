## Interactive views with `globalThis.tonk`

`<tonk-concept>` covers lists and grids where every row fits a
`<template>`. When you need a form, a button, or any UI whose logic
doesn't fit the row-template model, drop into JavaScript and use the
`globalThis.tonk` API the host injects into every view iframe.

Three methods, all return Promises. No handshake, no initialisation —
call them at the top of your script:

- `tonk.query(body)` — one-shot query. Resolves to an array of rows.
- `tonk.subscribe(body)` — live query. Resolves to a
  `ReadableStream` whose chunks are arrays of rows, one per branch
  change. Cancel the stream to unsubscribe.
- `tonk.evaluate(body, transact = true)` — run a notation document
  (assertions, queries, retractions). Resolves to the evaluate result.

The `body` for `query` / `subscribe` is the same `ConceptQuery` JSON
shape `/api/.../query` accepts. The `body` for `evaluate` is a string
of notation — the same syntax `slide eval` consumes.

You don't pass a repo or branch — the iframe is already scoped to one.

### Reading data

```yaml
view!: &person-counter
  body: |
    <p>People online: <span id="count">…</span></p>
    <script type="module">
      const stream = await tonk.subscribe({
        terms: { this: "?p" },
        predicate: { with: { name: "?n" } },
      });
      try {
        for await (const rows of stream) {
          document.getElementById("count").textContent = rows.length;
        }
      } catch (err) {
        console.error("count subscription failed:", err);
      }
    </script>
```

Cancel a subscription by calling `stream.cancel()` (or by aborting
the reader). The unsubscribe envelope to the worker is posted
automatically.

For a one-shot read:

```js
const rows = await tonk.query({
  terms:     { this: "?p" },
  predicate: { with: { name: "?n" } },
});
```

### Writing data

`evaluate` accepts any notation document — including assertions that
write claims:

```yaml
view!: &add-person
  body: |
    <form id="add">
      <input name="name" placeholder="Name" required>
      <button>Add</button>
    </form>
    <script type="module">
      document.getElementById("add").addEventListener("submit", async (e) => {
        e.preventDefault();
        const name = e.target.name.value;
        await tonk.evaluate(`person!:\n  name: "${name}"\n`);
        e.target.reset();
      });
    </script>
```

Notation strings let you assert, query, and retract in one document,
exactly as you would with `slide eval`. Quote string values so the
parser doesn't read them as symbols.

### Errors

Bad bodies and worker-side failures surface as Promise rejections —
for `subscribe`, errors raise on the stream and propagate out of the
`for await` loop. Treat them like any other JS error:

```js
try {
  await tonk.evaluate(`person!:\n  name: ${unquoted}\n`);
} catch (e) {
  alert(`couldn't save: ${e.message}`);
}
```

If `globalThis.tonk` is undefined entirely, the view is being mounted
outside a host iframe and the bridge module never loaded.

### Choosing between `<tonk-concept>` and `tonk.*`

| Reach for `<tonk-concept>` when | Reach for `globalThis.tonk` when |
|----------------------------------|----------------------------------|
| You're rendering rows from a query | You're handling user input |
| Each row fits a `<template>` | The UI doesn't decompose into rows |
| No write path needed | You need `evaluate` (assertions, retractions) |
| You want zero JS | You're already writing JS for other reasons |

They compose: a view can declaratively render a list with
`<tonk-concept>` *and* run a script that calls `tonk.evaluate` to
append to that list. The subscription `<tonk-concept>` holds will
re-render automatically once the write commits.
