// Hand the draft's editor the notebooks the directory rendered, as
// `{ title, href }`: a property, read on every keystroke.
(self) => {
  const prose = self.querySelector('notebook-editor[draft] prose-editor');
  if (!prose) return;
  prose.candidates = [...self.querySelectorAll('.notebook-index__item')].flatMap((row) => {
    const title = (row.querySelector('.notebook-index__title')?.textContent ?? '').trim();
    const href = row.getAttribute('href') ?? '';
    return title && href ? [{ title, href }] : [];
  });
}
