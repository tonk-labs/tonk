// Read an entry's spilled value (`tree/value` of its `asset:` entity)
// and keep it on the entry: its size, and its bytes as text when they
// are text, else as hex. Asked for by the reader, since a value can be
// large and may have to be fetched.
async (self, entry) => {
  entry.value = { loading: true };
  self.schedule();
  try {
    const [row] = await self.step('tree/value', { of: String(entry.spill) }, ['size', 'bytes']);
    if (!row) throw new Error('this replica has no such value');
    const bytes = self.bytes(row.bytes);
    const head = Uint8Array.from(bytes.slice(0, 4096));
    let text;
    try {
      text = new TextDecoder('utf-8', { fatal: true }).decode(head);
    } catch {
      text = bytes.slice(0, 1024).map((byte) => byte.toString(16).padStart(2, '0')).join(' ');
    }
    if (bytes.length > head.length) text += '\n…';
    entry.value = { size: Number(row.size ?? bytes.length), text };
  } catch (error) {
    entry.value = { error: String(error?.message ?? error) };
  }
  self.schedule();
}
