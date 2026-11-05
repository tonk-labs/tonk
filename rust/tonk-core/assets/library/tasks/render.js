(self) => {
  // One row per invocation, newest first: where it got to, what it is
  // doing or why it failed, and when. An open row renders the invocation
  // through its command's own `status` view.
  if (!self.__list) {
    const title = document.createElement('h1');
    title.textContent = 'Tasks';
    self.__list = document.createElement('ol');
    self.__list.addEventListener('click', (event) => {
      const row = event.target.closest('button.row');
      if (!row) return;
      const id = row.dataset.invocation;
      if (self.__open.has(id)) self.__open.delete(id);
      else self.__open.add(id);
      self.render();
    });
    self.__empty = document.createElement('p');
    self.__empty.className = 'empty';
    self.__empty.textContent = 'Nothing has run here lately. Commands you run from the command line show up here.';
    self.append(title, self.__empty, self.__list);
  }
  const words = {
    'case:active': 'running',
    'case:done': 'done',
    'case:failed': 'failed',
    'case:aborted': 'stopped',
    'case:unhandled': 'not handled',
  };
  const rows = [...(self.__rows || new Map()).entries()]
    .sort(([, a], [, b]) => (b.started || 0) - (a.started || 0));
  self.__empty.hidden = rows.length > 0;
  const branch = self.getAttribute('with');
  self.__list.textContent = '';
  for (const [id, fields] of rows) {
    const item = document.createElement('li');
    const name = String(fields.case || '').replace(/^case:/, '');
    item.dataset.case = name;
    const row = document.createElement('button');
    row.className = 'row';
    row.dataset.invocation = id;
    row.setAttribute('aria-expanded', String(self.__open.has(id)));
    const part = (className, text) => {
      const span = document.createElement('span');
      span.className = className;
      span.textContent = text;
      return span;
    };
    const what = fields.case === 'case:failed'
      ? fields.outputStatus || 'it failed'
      : fields.stateStatus || (fields.concept || '').replace(/^tonk:/, '') || id;
    const at = new Date(fields.finished || fields.started || 0);
    row.append(
      part('case', words[fields.case] || name),
      part('what', what),
      part('when', at.toLocaleTimeString()),
    );
    item.append(row);
    if (self.__open.has(id) && fields.concept) {
      const detail = document.createElement('div');
      detail.className = 'detail';
      const display = document.createElement('tonk-display');
      display.setAttribute('with', branch);
      display.setAttribute('entity', id);
      display.setAttribute('model', fields.concept);
      display.setAttribute('view', 'status');
      detail.append(display);
      item.append(detail);
    }
    self.__list.append(item);
  }
}
