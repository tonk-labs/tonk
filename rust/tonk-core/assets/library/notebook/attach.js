// Create the editor, listen to it, and start bringing it in line with the
// rows.
//
// A DRAFT notebook (the index's) has no notebook behind it yet: its
// heading is the switcher, which suggests the notebooks a title matches,
// opens the one picked, or creates the one named. A saved notebook's
// heading only renames it, so renaming onto a title that exists never
// navigates away from the document being edited.
(self) => {
  const state = self.__notebook;
  const prose = document.createElement('prose-editor');
  // No frame and no focus ring: the active-block band already shows where
  // the caret is. Inline, so it beats the editor's own `:focus-within`.
  prose.setAttribute('style', 'border: none; box-shadow: none; border-radius: 0; background: transparent;');
  prose.setAttribute('auto-focus', '');
  if (state.draft) {
    prose.setAttribute('switcher', '');
    prose.setAttribute('value', '# ');
    prose.setAttribute('placeholder', 'Name a notebook to open or create it...');
  } else {
    prose.setAttribute('placeholder', 'Write, and add a ```dialog-yaml block...');
    // Somewhere you came to write: focused, caret at the end.
    prose.setAttribute('caret', 'end');
  }
  state.prose = prose;
  self.listen();
  self.append(prose);
  self.observe();
}
