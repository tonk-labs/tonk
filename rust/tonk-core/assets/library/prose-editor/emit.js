// Composed, so it crosses the shadow boundary to listeners on the page.
(self, name, detail) => {
  self.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));
}
