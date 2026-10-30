// Host supplies the route context and presentation options. Rendering stays
// in the stock Tonk guest runtime, including installed space views.
addEventListener('message', (e) => {
  if (e.source !== parent || e.data?.type !== 'tonk-mount') return;
  window.__tonkHideFab = e.data.hideFab === true;
  if (window.__tonkHideFab) {
    const style = document.createElement('style');
    style.textContent = 'tonk-fab{display:none!important}';
    document.head.append(style);
  }
  // A host may name entities to display, each with its concept's own
  // display, in place of the route's site.
  const items = Array.isArray(e.data.items)
    ? e.data.items
    : [{ entity: e.data.site, model: e.data.model }];
  document.body.replaceChildren(
    ...items.map((item) => {
      const display = document.createElement('tonk-display');
      display.setAttribute('entity', item.entity);
      display.setAttribute('model', item.model);
      if (item.view) display.setAttribute('view', item.view);
      display.setAttribute('with', 'main@' + e.data.subject);
      return display;
    }),
  );
  if (!Array.isArray(e.data.items)) return;
  // Displays size to their content, which only this document can measure.
  const style = document.createElement('style');
  style.textContent = 'html,body{min-height:0!important}body{padding:8px;display:flex;flex-direction:column;gap:12px}';
  document.head.append(style);
  let reported = 0;
  new ResizeObserver(() => {
    const height = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (height === reported) return;
    reported = height;
    parent.postMessage({ type: 'tonk-display-size', height }, '*');
  }).observe(document.documentElement);
});
