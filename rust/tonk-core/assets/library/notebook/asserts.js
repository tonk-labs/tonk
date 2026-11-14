// Whether `text` asserts anything: whether one of its top-level heads ends
// in `!` (`person!:`, `concept!: &name`). Only the shape is read; a cell
// asks this of a buffer the language server has already found clean.
(self, text) => {
  for (const line of String(text ?? '').split('\n')) {
    const head = line.match(/^(["']?)([^\s#'"-][^:]*?)\1\s*:(?:\s|$)/);
    if (head && head[2].endsWith('!')) return true;
  }
  return false;
}
