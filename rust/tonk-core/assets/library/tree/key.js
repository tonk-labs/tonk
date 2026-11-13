// Append a key's parts to `parent` as coloured chips, each with a tooltip
// naming its part.
//
// In the outline (`at` given: the key's raw `hex` and its siblings'
// `prev` and `next`) the key is dimmed past its routing pivot, the byte
// it has to stay bright through to tell it apart from both neighbours:
// the bytes up to there decide where a lookup branches, the rest is
// noise for this row's place. Long parts are elided. Without `at` (the
// detail pane) every part shows whole.
(self, parent, parts, at) => {
  const kit = self.kit();
  const state = self.__tree;
  const components = self.components(parts);
  const pivot = at ? self.pivot(at.hex, at.prev, at.next) : null;
  // Keep the head and the tail of anything long, a `did:key:…` say.
  const elide = (text) => {
    const chars = [...text];
    return chars.length <= 18 ? text : `${chars.slice(0, 12).join('')}…${chars.slice(-4).join('')}`;
  };
  components.forEach((component, index) => {
    let className = `key-seg ${component.className}`;
    if (index === 0) className += ' seg-index-type';
    if (at && pivot !== null && component.bytes[0] > pivot) className += ' dim';
    const id = `kseg-${++state.seq}`;
    const chip = kit.el('span', className);
    chip.id = id;
    chip.append(kit.el('span', 'seg-text', at ? elide(component.text) : component.full));
    const tip = kit.el('wa-tooltip', null, component.label);
    tip.setAttribute('for', id);
    parent.append(chip, tip);
  });
}
