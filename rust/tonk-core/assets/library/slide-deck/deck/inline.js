(self, text) => {
  const kept = [];
  const hold = (html) => `\u0000${kept.push(html) - 1}\u0000`;
  let t = String(text)
    .replace(/`([^`]+)`/g, (m, code) => hold(`<code>${self.escape(code)}</code>`))
    .replace(/\\([!-\/:-@\[-`{-~])/g, (m, c) => hold(self.escape(c)));
  t = self.escape(t)
    .replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (m, alt, src) => hold(`<img alt="${alt}" src="${self.safeUrl(src)}">`))
    .replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, label, href) => `<a href="${self.safeUrl(href)}">${label}</a>`)
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/__([^_]+)__/g, '<strong>$1</strong>')
    .replace(/(^|[^*\w])\*([^*\s][^*]*?)\*(?!\w)/g, '$1<em>$2</em>')
    .replace(/(^|[^_\w])_([^_\s][^_]*?)_(?!\w)/g, '$1<em>$2</em>')
    .replace(/~~([^~]+)~~/g, '<del>$1</del>');
  return t.replace(/\u0000(\d+)\u0000/g, (m, k) => kept[k]);
}
