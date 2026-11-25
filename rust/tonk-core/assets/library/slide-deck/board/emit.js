(self, type, detail) => {
  self.dispatchEvent(new CustomEvent(type, { bubbles: true, detail }));
}
