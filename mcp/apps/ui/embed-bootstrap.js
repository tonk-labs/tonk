// Host supplies the route context and presentation options. Rendering stays
// in the stock Tonk guest runtime, including installed space views.
// The agent app's theme, which the host passes on mount and again
// whenever it changes.
addEventListener('message', (e) => {
  if (e.source !== parent || e.data?.type !== 'tonk-theme') return;
  window.__tonkSetTheme?.(e.data.theme);
});
// How big the host shows the view: strip, card, carousel or full. Inline
// (anything but full), a view drops its own heading, since the host's
// line names it: an element marked `data-tonk-heading` is hidden.
const sizeStyle = document.createElement('style');
sizeStyle.textContent =
  'html[data-tonk-size]:not([data-tonk-size="full"]) [data-tonk-heading]{display:none!important}';
document.head.append(sizeStyle);
function setSize(size) {
  if (!['strip', 'card', 'carousel', 'full'].includes(size)) return;
  document.documentElement.dataset.tonkSize = size;
}
addEventListener('message', (e) => {
  if (e.source !== parent || e.data?.type !== 'tonk-size') return;
  setSize(e.data.size);
});
addEventListener('message', (e) => {
  if (e.source !== parent || e.data?.type !== 'tonk-mount') return;
  if (e.data.theme) window.__tonkSetTheme?.(e.data.theme);
  if (e.data.size) setSize(e.data.size);
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
  // The host sizes the view to its content, which only this document
  // can measure; the relay passes the height up.
  const style = document.createElement('style');
  style.textContent = 'html,body{height:auto!important;min-height:0!important}';
  document.head.append(style);
  let reported = 0;
  new ResizeObserver(() => {
    const height = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (height === reported) return;
    reported = height;
    parent.postMessage({ type: 'tonk-view-size', height }, '*');
  }).observe(document.documentElement);
});
