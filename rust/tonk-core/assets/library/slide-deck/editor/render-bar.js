(self) => {
  const s = self.__edit;
  if (!s) return;
  const options = [['prose-editor', 'Rich'], ['code-editor', 'Source'], ['textarea', 'Plain']]
    .filter(([tag]) => tag === 'textarea' || customElements.get(tag));
  if (options.length < 2) { s.bar.replaceChildren(); return; }
  s.bar.replaceChildren(...options.map(([tag, label]) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = label;
    button.setAttribute('aria-pressed', String(tag === s.tag));
    button.addEventListener('click', () => { if (s.tag !== tag) self.mountWith(tag); });
    return button;
  }));
}
