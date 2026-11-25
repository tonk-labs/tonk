(self, event) => {
  const path = event.composedPath?.() ?? [];
  const editing = path.some((node) => node?.isContentEditable || /^(INPUT|TEXTAREA|SELECT|PROSE-EDITOR|CODE-EDITOR|SLIDE-EDITOR)$/.test(node?.tagName ?? ''));
  if (editing) return false;
  return !self.hasAttribute('preview') || path.includes(self);
}
