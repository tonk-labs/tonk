(self, step) => {
  const links = [...self.querySelectorAll('.vc-entry a[data-revision]')];
  if (!links.length) return;
  const current = links.findIndex((link) => link.closest('.vc-entry')?.getAttribute('aria-current') === 'true');
  const next = links[Math.min(links.length - 1, Math.max(0, current + step))];
  if (!next || next === links[current]) return;
  self.__refocus = next.dataset.revision;
  next.click();
}
