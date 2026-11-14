// Create the notebook named `title`, carrying the draft's `body`: the
// page mints its entity and writes it, the title and the draft's blocks in
// one commit (the library's rules place the blocks), and opens it once the
// commit landed, so its page never reads before its facts exist.
//
// Minted, not derived: an anchor-less write takes its entity from its body
// digest, which the page never learns, and two creates from one head would
// derive the same one from anything a rule can read.
async (self, title, body) => {
  const draft = self.querySelector('notebook-editor[draft]');
  if (!draft) return;
  const id = globalThis.crypto?.randomUUID?.() ?? Date.now().toString(16);
  const entity = `notebook:${id}`;
  try {
    await draft.evaluate(draft.notation().create(entity, title, body), true);
  } catch (error) {
    console.error('notebook create failed:', error?.message ?? error);
    return;
  }
  self.navigate(`/notebook/${entity}`);
}
