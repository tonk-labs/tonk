// The rules that draw the active block's band over the document's
// top-level nodes `[start, length]`: quieter than a focused cell ("you are
// here", not "this is selected"), bleeding into the gutter on both sides,
// with only the run's outer corners rounded so several nodes read as one.
(self, [start, length]) => {
  const node = (index) => `.md-doc > :nth-child(${index + 1})`;
  const nodes = Array.from({ length }, (_, offset) => node(start + offset));
  return `${nodes.join(',\n')} {
  background: rgba(127, 127, 127, 0.14);
  background: color-mix(in srgb, currentColor 12%, transparent);
  box-shadow: -0.6rem 0 0 0 rgba(127, 127, 127, 0.14), 0.6rem 0 0 0 rgba(127, 127, 127, 0.14);
  box-shadow: -0.6rem 0 0 0 color-mix(in srgb, currentColor 12%, transparent),
    0.6rem 0 0 0 color-mix(in srgb, currentColor 12%, transparent);
}
${node(start)} { border-start-start-radius: 0.25rem; border-start-end-radius: 0.25rem; }
${node(start + length - 1)} { border-end-start-radius: 0.25rem; border-end-end-radius: 0.25rem; }
`;
}
