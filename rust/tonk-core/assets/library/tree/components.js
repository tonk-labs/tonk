// The worker's decoded key `parts` (`{kind, text, hex}`) as chips to draw:
// `{label, className, text, full, bytes}`. The decoding is the worker's;
// this only names and colours what it found.
//
// Colours follow the part: entity circle-blue, attribute triangle-yellow,
// value and its type square-red, the structural bytes (index tag, version,
// lower bound) neutral. The index chip keeps one letter of its ordering
// (`E`), since the parts after it are coloured by exactly that; its
// tooltip, like the value-type chip's, names what its byte says rather
// than repeating the byte. Control characters show as glyphs (`␀`, `·`),
// so a separator byte is visible. `bytes` is where each part sits in the
// raw key, `[start, end)`, from its hex length, for the routing pivot.
(self, parts) => {
  const PARTS = {
    index: ['seg-index-type', 'Index'],
    entity: ['seg-entity', 'Entity'],
    attribute: ['seg-attribute', 'Attribute'],
    vtype: ['seg-vtype', 'Value type'],
    value: ['seg-value', 'Value'],
    spill: ['seg-value', 'Spilled value (hash reference)'],
    origin: ['seg-index-type', 'Revision origin'],
    edition: ['seg-index-type', 'Revision edition'],
    blob: ['seg-value', 'Blob (content-addressed)'],
    min: ['seg-index-type', 'Leftmost subtree (no lower bound)'],
  };
  const glyphs = (text) => [...text]
    .map((c) => (c === '\0' ? '␀' : /[\u0000-\u001f\u007f-\u009f]/.test(c) ? '·' : c))
    .join('');
  const hexLength = (hex) => Math.floor((hex.startsWith('0x') ? hex.slice(2) : hex).length / 2);
  let offset = 0;
  return parts.map((part) => {
    const [className, name] = PARTS[part.kind] ?? ['seg-unknown', 'Key'];
    let label = name;
    if (part.kind === 'index') {
      label = part.text ? `${part.text[0].toUpperCase()}${part.text.slice(1)} index` : 'Index';
    } else if (part.kind === 'vtype') {
      label = `Value type: ${part.text}`;
    }
    const text = part.kind === 'index' ? (part.text[0] ?? '').toUpperCase() : glyphs(part.text);
    const length = Math.max(1, hexLength(part.hex));
    const bytes = [offset, offset + length];
    offset += length;
    return { label, className, text, full: text, bytes };
  });
}
