(self, lang) => {
  const string = String.raw`"(?:[^"\\]|\\.)*"`;
  const number = String.raw`\b\d[\d_]*(?:\.\d+)?\b`;
  const rust = [
    ['comment', String.raw`//[^\n]*|/\*[\s\S]*?\*/`],
    ['attr', String.raw`#!?\[[^\]\n]*\]`],
    ['string', string],
    ['keyword', String.raw`\b(?:as|async|await|break|const|continue|crate|dyn|else|enum|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|type|unsafe|use|where|while|true|false|Some|None|Ok|Err)\b`],
    ['macro', String.raw`\b[a-z_]\w*!`],
    ['type', String.raw`\b[A-Z]\w*`],
    ['number', number],
    ['call', String.raw`\b[a-z_]\w*(?=\s*\()`],
  ];
  const yaml = [
    ['comment', String.raw`(?:^|(?<=\s))#[^\n]*`],
    ['string', string + `|'[^'\\n]*'`],
    ['key', String.raw`[\w.\/!&?=<>-]+(?=:(?:\s|$))`],
    ['var', String.raw`\?[\w\/.-]+`],
    ['anchor', String.raw`&[\w\/.-]+`],
    ['keyword', String.raw`\b(?:true|false|null)\b`],
    ['number', number],
  ];
  const clojure = [
    ['comment', String.raw`;[^\n]*`],
    ['string', string],
    ['head', String.raw`(?<=\()[\w.\/!?*<>=+-]+`],
    ['arg', String.raw`[\w.\/-]+:(?=\s)`],
    ['keyword', String.raw`(?<![\w])\:[\w.\/!?*-]+`],
    ['var', String.raw`\?[\w\/.-]+`],
    ['number', number],
  ];
  const table = { rust, rs: rust, yaml, yml: yaml, 'dialog-yaml': yaml, clojure, clj: clojure, edn: clojure, lisp: clojure };
  return table[lang] ?? null;
}
