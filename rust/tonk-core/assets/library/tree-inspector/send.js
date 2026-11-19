// Send one of the element's events (`tree-explore`, `tree-open`,
// `tree-close`); the console view binds each to its command.
(self, type, detail) => {
  self.dispatchEvent(new CustomEvent(type, {
    detail: { ...detail, time: Date.now() },
    bubbles: true,
    composed: true,
  }));
}
