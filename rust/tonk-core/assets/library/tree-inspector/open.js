// Read a node and what it holds, and show it open: what the node is
// (`tree/node`), then its spans if it is an index (`tree/span`) or its
// entries if it is a segment (`tree/entry`). The three ask of the same
// handle, and the worker decodes the node once. Nothing below the node
// is read until it is opened in its turn.
async (self, node) => {
  const state = self.__tree;
  let known = state.nodes.get(node);
  if (!known) {
    known = { node, open: true, loading: false, info: null, spans: null, entries: null, shown: 100, error: null };
    state.nodes.set(node, known);
  }
  known.open = true;
  if (known.loading || known.spans || known.entries) {
    self.schedule();
    return;
  }
  known.loading = true;
  known.error = null;
  self.schedule();
  const number = (value) => (typeof value === 'number' ? value : Number(value ?? 0));
  try {
    const [info] = await self.step('tree/node', { of: node }, [
      'kind', 'size', 'count', 'scale', 'novelty', 'version', 'fanout-exponent',
      'max-separator', 'inline-n', 'spill-prefix', 'max-segment',
      'frame-ceiling-factor', 'anchor-selector', 'entry-overhead',
      'key-overhead', 'link-overhead',
    ]);
    if (!info) throw new Error('this replica has no such node');
    known.info = info;
    if (info.kind === 'index') {
      const spans = await self.step('tree/span', { of: node }, [
        'span', 'at', 'node', 'separator', 'until', 'scale', 'rank', 'novelty',
      ]);
      known.spans = spans
        .map((span) => ({
          ...span,
          at: number(span.at),
          node: String(span.node ?? ''),
          separator: self.bytes(span.separator),
          until: self.bytes(span.until),
        }))
        .sort((a, b) => a.at - b.at);
      for (const span of known.spans) self.label('dialog/separator-part', span.separator);
    } else {
      const entries = await self.step('tree/entry', { of: node }, [
        'entry', 'at', 'key', 'rank', 'state', 'retraction', 'origin', 'edition',
        'cause', 'collapsed', 'supersedes', 'spill', 'blob', 'blob-size', 'blob-version',
      ]);
      known.entries = entries
        .map((entry) => ({ ...entry, at: number(entry.at), key: self.bytes(entry.key) }))
        .sort((a, b) => a.at - b.at);
      for (const entry of known.entries.slice(0, known.shown)) self.label('dialog/key-part', entry.key);
    }
  } catch (error) {
    known.error = String(error?.message ?? error);
  }
  known.loading = false;
  self.schedule();
}
