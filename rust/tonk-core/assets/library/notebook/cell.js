// Wire one fence as a cell: its buffer on the language server, a result
// under it, and a run whenever the server finds the buffer clean.
//
// The buffer URI (`tonk-buffer:///<repo>/<branch>/notebook-<id>`) scopes
// completion and diagnostics to this notebook's branch; <code-editor>
// opens it on the language server itself once it has a `source`, and
// every verdict arrives as its `diagnostics` event.
//
// A clean query runs as a dry run on every frame: a query has no effect. A
// cell that asserts is never run by typing. It offers the bolt instead,
// and runs, committing, on a click or on Cmd/Ctrl+Enter or Shift+Enter
// (the editor's `run`).
(self, wrapper, editor, id) => {
  const { context } = self.__notebook;
  editor.setAttribute('source', `tonk-buffer:///${context.repo}/${context.branch}/notebook-${id}`);
  const slot = document.createElement('div');
  slot.className = 'notebook-cell-result';
  wrapper.append(slot);
  const cell = { editor, slot, running: false };
  const show = (failure, response) => {
    slot.innerHTML = self.result(failure, response, context.with);
  };
  const commit = async (body) => {
    try {
      show(null, await self.evaluate(body, true));
    } catch (error) {
      show(error?.message ?? String(error), null);
    }
  };
  // Tab inside a cell is the editor's (accept a completion, else indent);
  // unhandled, it would move focus out of the document.
  editor.addEventListener('keydown', (event) => {
    if (event.key === 'Tab') event.preventDefault();
  });
  editor.addEventListener('diagnostics', async (event) => {
    if ((event.detail?.errorCount ?? 0) > 0) return;
    const body = event.detail?.value ?? '';
    if (!body.trim()) {
      slot.innerHTML = '';
      return;
    }
    if (self.asserts(body)) {
      if (slot.querySelector('.evaluate-play')) return;
      slot.innerHTML =
        '<div class="notebook-cell-held"><wa-button type="button" class="evaluate-play is-visible" ' +
        'variant="neutral" appearance="filled" size="small" pill title="Run this cell, it writes (Cmd/Ctrl+Enter)">' +
        '<wa-icon name="bolt" variant="solid"></wa-icon></wa-button></div>';
      slot.querySelector('.evaluate-play').addEventListener('click', (click) => {
        click.preventDefault();
        click.stopPropagation();
        commit(editor.value);
      });
      return;
    }
    // A burst of frames must not stack runs, nor a late reply overwrite a
    // newer one.
    if (cell.running) return;
    cell.running = true;
    try {
      show(null, await self.evaluate(body, false));
    } catch (error) {
      show(error?.message ?? String(error), null);
    } finally {
      cell.running = false;
    }
  });
  editor.addEventListener('run', (event) => {
    event.preventDefault();
    event.stopPropagation();
    commit(editor.value);
  });
  return cell;
}
