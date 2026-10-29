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
  // A view mounts one display of the space on its own; otherwise the site
  // mounts the route the path matched.
  const view = e.data.view;
  const display = document.createElement('tonk-display');
  if (view) {
    display.setAttribute('model', view.model);
    if (view.entity) display.setAttribute('entity', view.entity);
    if (view.facet) display.setAttribute('view', view.facet);
  } else {
    display.setAttribute('entity', e.data.site);
    display.setAttribute('model', e.data.model);
  }
  display.setAttribute('with', 'main@' + e.data.subject);
  document.body.replaceChildren(display);
});
