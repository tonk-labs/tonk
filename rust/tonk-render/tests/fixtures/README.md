# Browser golden fixtures

These are the exact `outerHTML` a real `<tonk-view>` produces in the
browser, captured via Chrome DevTools by driving the component with a
known template and known conclusions:

```js
const host = document.createElement('tonk-view');
host.innerHTML = '<template>' + TEMPLATE + '</template>';
document.body.appendChild(host);            // connectedCallback snapshots the template
host.draw(FRAME);                            // FRAME = [{this, fields}, ...]
host.outerHTML                               // <- captured here
```

The `compat` test (in `tests/compat.rs`) renders the same TEMPLATE +
FRAME through `tonk-render` and asserts the output matches the golden
after normalization.

## Normalization

The browser renderer keeps state for in-place updates, which leaves
two artifacts the one-shot SSR renderer does not emit:

- the **`<tonk-view>` host wrapper** (the custom element shell, not
  part of the rendered view);
- **anchor comments** `<!--tonk-repeat-->` and `<!--tonk-iter:FIELD-->`
  that mark where the repeat / iteration elements were, used as
  insertion points across frames.

The normalizer strips both. After that, the rendered markup (elements,
attributes incl. the `with=` repeat stamp, text, escaping) is
byte-identical between the browser and `tonk-render`.
