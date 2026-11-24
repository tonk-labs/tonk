(self, markdown) => {
  const lines = String(markdown).replace(/\r\n?/g, '\n').split('\n');
  const blank = (line) => line === undefined || !line.trim();
  const item = /^\s*([-*+]|\d+[.)])\s+(.*)$/;
  const kind = (m) => (/\d/.test(m[1]) ? 'ol' : 'ul');
  const separator = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;
  const table = (k) => lines[k]?.includes('|') && separator.test(lines[k + 1] ?? '');
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    let m;
    if (blank(line)) { i++; continue; }
    if ((m = line.match(/^\s{0,3}(`{3,}|~{3,})\s*([\w+-]*)/))) {
      const fence = m[1];
      const lang = m[2].toLowerCase();
      const code = [];
      i++;
      while (i < lines.length && !(lines[i].trim().startsWith(fence) && !lines[i].trim().replace(/[`~]/g, ''))) code.push(lines[i++]);
      i++;
      out.push(lang === 'svg' ? self.diagram(code.join('\n')) : `<pre><code>${self.highlight(lang, code.join('\n'))}</code></pre>`);
    } else if ((m = line.match(/^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/))) {
      out.push(`<h${m[1].length}>${self.inline(m[2])}</h${m[1].length}>`);
      i++;
    } else if (table(i)) {
      const cells = (row) => row.trim().replace(/^\|/, '').replace(/\|$/, '').split(/(?<!\\)\|/).map((cell) => cell.trim());
      const align = cells(lines[i + 1]).map((c) => (c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : c.startsWith(':') ? 'left' : ''));
      const cell = (tag, text, k) => `<${tag}${align[k] ? ` style="text-align:${align[k]}"` : ''}>${self.inline(text)}</${tag}>`;
      const head = cells(line).map((text, k) => cell('th', text, k)).join('');
      const rows = [];
      i += 2;
      while (i < lines.length && !blank(lines[i]) && lines[i].includes('|')) rows.push(`<tr>${cells(lines[i++]).map((text, k) => cell('td', text, k)).join('')}</tr>`);
      out.push(`<table><thead><tr>${head}</tr></thead><tbody>${rows.join('')}</tbody></table>`);
    } else if (/^\s{0,3}>/.test(line)) {
      const quote = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i])) quote.push(lines[i++].replace(/^\s{0,3}>\s?/, ''));
      out.push(`<blockquote>${self.toHtml(quote.join('\n'))}</blockquote>`);
    } else if ((m = line.match(item))) {
      const tag = kind(m);
      const items = [];
      while (i < lines.length) {
        const next = lines[i].match(item);
        if (next && kind(next) === tag) { items.push([next[2]]); i++; continue; }
        if (blank(lines[i])) {
          let j = i;
          while (j < lines.length && blank(lines[j])) j++;
          const after = lines[j]?.match(item);
          if (after && kind(after) === tag) { i = j; continue; }
          break;
        }
        if (/^\s+\S/.test(lines[i]) && items.length) { items.at(-1).push(lines[i++].trim()); continue; }
        break;
      }
      out.push(`<${tag}>${items.map((text) => `<li>${self.inline(text.join(' '))}</li>`).join('')}</${tag}>`);
    } else {
      const para = [];
      while (i < lines.length && !blank(lines[i]) && !/^\s{0,3}(`{3,}|~{3,}|#{1,6}\s|>)/.test(lines[i]) && !item.test(lines[i]) && !table(i)) para.push(lines[i++].trim());
      if (!para.length) para.push(lines[i++].trim());
      out.push(`<p>${para.map((text) => self.inline(text)).join('<br>')}</p>`);
    }
  }
  return out.join('\n');
}
