(self) => {
  // One row per invocation, newest first: where it got to, what it is
  // doing or why it failed, and when it was run. An open row renders what
  // the command produced, when it produced something.
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
  // Where an invocation got to: a failure whatever else came, then an
  // answer or an output (a command a rule handled has only that), then
  // dialog's unhandled, which a rule-handled command also carries.
  const caseOf = (fields) => {
    if (fields.failed) return 'case:failed';
    if (fields.answered || fields.output) return 'case:done';
    if (fields.unhandled) return 'case:unhandled';
    return 'case:active';
  };
  const rows = [...(self.__rows || new Map()).entries()]
    .sort(([, a], [, b]) => (b.invoked || 0) - (a.invoked || 0));
  self.__empty.hidden = rows.length > 0;
  const branch = self.getAttribute('with');
  self.__list.textContent = '';
  for (const [id, fields] of rows) {
    const item = document.createElement('li');
    const now = caseOf(fields);
    const name = now.replace(/^case:/, '');
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
    // The command by what it answers to, then what it is doing or why it
    // failed.
    const command = fields.name || 'a command';
    const detail = now === 'case:failed' ? fields.failed : now === 'case:active' ? fields.status : '';
    const what = detail ? `${command}: ${detail}` : command;
    const at = new Date(fields.invoked || 0);
    row.append(
      part('case', words[now] || name),
      part('what', what),
      part('when', at.toLocaleTimeString()),
    );
    item.append(row);
    if (self.__open.has(id) && fields.output) {
      const detail = document.createElement('div');
      detail.className = 'detail';
      const display = document.createElement('tonk-display');
      display.setAttribute('with', branch);
      display.setAttribute('entity', fields.output);
      detail.append(display);
      item.append(detail);
    }
    self.__list.append(item);
  }
}
