// Append a fresh cell, sealing the one before it.
//
// A cell's buffer URI names the branch (`tonk-buffer:///<repo>/<branch>/
// scratch-<n>`), so the language server completes and checks against it.
// Each frame the server sends is the cue to run: a clean one runs the
// buffer as a dry run and shows what it answers, or would change. The
// bolt shows only while the buffer is clean and asserts something, since
// that is all a commit would add to a dry run.
(self, focus) => {
  const state = self.__inspector;
  const { cells, context } = state;
  const previous = cells.lastElementChild;
  if (previous) {
    previous.classList.add('cell-sealed');
    previous.querySelector('code-editor')?.setAttribute('readonly', '');
  }
  const id = state.next++;
  const form = document.createElement('form');
  form.className = 'branch-yaml-query';
  form.innerHTML =
    '<div class="evaluate-editor">' +
    '<code-editor language="dialog-yaml" active-line></code-editor>' +
    '<wa-button class="evaluate-play" type="button" variant="neutral" appearance="filled" size="small" pill' +
    ' title="Submit transaction (Shift+Enter)"><wa-icon name="bolt" variant="solid"></wa-icon></wa-button>' +
    '</div><div class="evaluate-slot"></div>';
  const editor = form.querySelector('code-editor');
  const play = form.querySelector('.evaluate-play');
  editor.setAttribute('source', `tonk-buffer:///${context.repo}/${context.branch}/scratch-${id}`);
  editor.setAttribute(
    'placeholder',
    id === 0
      ? 'person:\n  this: ?alice\n  name: "Alice"\n\n# or assert with `!`:\n# person!: &alice\n#   name: "Alice"'
      : '',
  );
  if (focus) editor.setAttribute('auto-focus', '');

  const cell = {
    form, editor, play,
    slot: form.querySelector('.evaluate-slot'),
    buffer: '', errors: 0, running: false, sealed: false,
  };
  cell.refresh = () => {
    const runnable = !cell.sealed && cell.errors === 0 && self.asserts(cell.buffer);
    play.classList.toggle('is-visible', runnable);
  };
  form.addEventListener('submit', (event) => event.preventDefault());
  editor.addEventListener('change', (event) => {
    cell.buffer = event.detail?.value ?? editor.value ?? '';
    cell.refresh();
  });
  editor.addEventListener('run', () => self.submit(cell));
  play.addEventListener('click', (event) => {
    event.preventDefault();
    event.stopPropagation();
    self.submit(cell);
  });
  editor.addEventListener('diagnostics', async (event) => {
    cell.errors = event.detail?.errorCount ?? 0;
    cell.refresh();
    if (cell.sealed || cell.errors > 0 || cell.running) return;
    const body = event.detail?.value ?? '';
    if (!body.trim()) return;
    try {
      const response = await self.evaluate(body, false);
      cell.slot.innerHTML = self.result(null, response);
    } catch {
      // A preview that fails says nothing; submitting shows why.
    }
  });
  cells.append(form);
  return cell;
}
