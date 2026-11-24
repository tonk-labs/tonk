(self, lang, code) => {
  const rules = self.grammar(lang);
  if (!rules) return self.escape(code);
  const pattern = new RegExp(rules.map(([, source]) => `(${source})`).join('|'), 'gm');
  let out = '';
  let last = 0;
  let m;
  while ((m = pattern.exec(code))) {
    if (!m[0]) { pattern.lastIndex++; continue; }
    const k = m.slice(1).findIndex((group) => group !== undefined);
    out += self.escape(code.slice(last, m.index)) + `<span class="t-${rules[k][0]}">${self.escape(m[0])}</span>`;
    last = m.index + m[0].length;
  }
  return out + self.escape(code.slice(last));
}
