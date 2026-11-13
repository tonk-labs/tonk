// A positive-integer attribute, or undefined when absent or malformed
// (the grid then keeps its default).
(self, name) => {
  const raw = self.getAttribute(name);
  if (raw === null) return undefined;
  const n = Number(raw);
  return Number.isInteger(n) && n >= 1 ? n : undefined;
}
