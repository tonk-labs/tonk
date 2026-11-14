// Go to `href` as a route change, not a page load: through the page's
// `tonk.navigate`, which a guest forwards to the frame that routes it. A
// page without one gets the link a guest relays anyway: an attached anchor,
// clicked.
(self, href) => {
  if (typeof window.tonk?.navigate === 'function') {
    window.tonk.navigate(href);
    return;
  }
  const link = document.createElement('a');
  link.href = href;
  link.hidden = true;
  self.append(link);
  link.click();
  link.remove();
}
