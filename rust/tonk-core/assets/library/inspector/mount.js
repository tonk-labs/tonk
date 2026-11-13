// Build the inspector for `context` (from `context()`), replacing whatever
// it showed: the branch diagnostics (a space's, not the profile's), the
// notice that stands in while code-editor is missing, and the first cell.
(self, context) => {
  const previous = self.__inspector;
  previous?.watch?.cancel();
  self.classList.add('tonk-inspector');
  if (!context) {
    self.__inspector = null;
    self.innerHTML =
      '<section class="error">no repository in context (give it with="branch@repo")</section>';
    return;
  }
  const notice = document.createElement('div');
  notice.className = 'inspector-requires';
  notice.hidden = true;
  notice.innerHTML =
    '<wa-callout variant="warning"><wa-icon slot="icon" name="triangle-exclamation"></wa-icon>' +
    'The inspector edits in <code>code-editor</code>, which this space does not have. ' +
    'Install the code-editor component and the cells below become editors.</wa-callout>';
  const cells = document.createElement('div');
  cells.className = 'branch-cells';
  const state = (self.__inspector = { context, notice, cells, panel: null, next: 0, watch: null });
  self.replaceChildren();
  if (!context.profile) {
    state.panel = self.panel();
    self.append(state.panel.host);
  }
  self.append(notice, cells);
  self.cell(true);
  self.require();
}
