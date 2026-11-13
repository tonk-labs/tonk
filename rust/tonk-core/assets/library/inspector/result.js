// A cell's result as HTML: the failure callout, when there is one, and
// what the evaluate `response` matched.
//
// Unchanged matches render once, with the revision as a badge, in three
// swappable views: listed notation, a grouped tree and a table per query.
// Matches the commit changed render as a before/after comparison of their
// notation. `concept:`, `command:` and `rule:` results draw their stored
// descriptor as the `concept!:` / `command!:` / `rule!:` that would write
// it, types and variables spelled as notation spells them.
(self, failure, response) => {
  const esc = (text) => String(text)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
  const sorted = (object) => Object.keys(object).sort().map((key) => [key, object[key]]);

  const scheme = (text) => /^[a-z][a-z0-9+-]*$/.test(text);
  const looksLikeUri = (text) => !/\s/.test(text) && (text.includes(':') || (text.includes('.') && text.includes('/')));
  // A bare-scheme URI (`keyword:`) is spelled `:keyword` in notation.
  const spellUri = (uri) => (uri.endsWith(':') && scheme(uri.slice(0, -1)) ? `:${uri.slice(0, -1)}` : uri);

  // A stored type as notation types it: by its built-in anchor (`text`,
  // `natural:64`), whether stored as its entity or by an older name.
  const LEGACY = {
    Bytes: 'bytes:', Entity: 'entity:', Boolean: 'boolean:', Text: 'text:',
    UnsignedInteger: 'natural:', SignedInteger: 'integer:', Float: 'float:',
    Record: 'record:', Symbol: 'symbol:', Number: 'number:',
  };
  const TYPES = new Set([
    ...Object.values(LEGACY),
    'natural:8', 'natural:16', 'natural:32', 'natural:64', 'natural:128',
    'integer:8', 'integer:16', 'integer:32', 'integer:64', 'integer:128',
    'float:32', 'float:64',
  ]);
  const typeAnchor = (stored) => {
    const uri = LEGACY[stored] ?? (TYPES.has(stored) ? stored : null);
    if (!uri) return stored;
    return uri.endsWith(':') ? uri.slice(0, -1) : uri;
  };
  const normalizeTypes = (value) => {
    if (Array.isArray(value)) return value.map(normalizeTypes);
    if (!isObject(value)) return value;
    const out = {};
    for (const [key, child] of Object.entries(value)) {
      out[key] = key === 'as' && typeof child === 'string' ? typeAnchor(child) : normalizeTypes(child);
    }
    return out;
  };
  // A rule's variable terms (`{"?": {"name": "x"}}`) as `?x`.
  const normalizeTerms = (value) => {
    if (Array.isArray(value)) return value.map(normalizeTerms);
    if (!isObject(value)) return value;
    const keys = Object.keys(value);
    if (keys.length === 1 && keys[0] === '?') {
      const name = value['?']?.name;
      return typeof name === 'string' ? `?${name}` : '?';
    }
    const out = {};
    for (const [key, child] of Object.entries(value)) out[key] = normalizeTerms(child);
    return out;
  };
  const parsed = (value) => {
    if (isObject(value)) return value;
    if (typeof value !== 'string') return null;
    try {
      const object = JSON.parse(value);
      return isObject(object) ? object : null;
    } catch {
      return null;
    }
  };
  const descriptor = (result, showTransient) => {
    const source = parsed(result.fields?.source);
    if (!source) return null;
    const map = normalizeTypes(source);
    if (showTransient && result.fields?.transient === true) map.transient = true;
    return map;
  };
  const ruleDefinition = (result) => {
    const outer = parsed(result.fields?.definition);
    if (!outer || !isObject(outer.rule)) return null;
    const rule = { ...outer.rule };
    // Rows older releases wrote carry a retract rule's head as `assert!`.
    if (typeof outer.polarity === 'string' && outer.polarity.toLowerCase() === 'retract' && 'assert!' in rule) {
      rule['retract!'] = rule['assert!'];
      delete rule['assert!'];
    }
    return normalizeTypes(normalizeTerms(rule));
  };

  const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  // Each leading zero byte is a `1`; the rest is the number in base 58.
  const base58 = (bytes) => {
    const zeros = bytes.findIndex((byte) => byte !== 0);
    if (zeros === -1) return '1'.repeat(bytes.length);
    const digits = [];
    for (const byte of bytes.slice(zeros)) {
      let carry = byte;
      for (let i = 0; i < digits.length; i++) {
        carry += digits[i] * 256;
        digits[i] = carry % 58;
        carry = Math.floor(carry / 58);
      }
      while (carry > 0) {
        digits.push(carry % 58);
        carry = Math.floor(carry / 58);
      }
    }
    return '1'.repeat(zeros) + digits.reverse().map((digit) => ALPHABET[digit]).join('');
  };
  // The revision's tree as `#<base58>`: the wire carries its hash as bytes.
  const treeDisplay = (tree) => {
    if (typeof tree === 'string' && tree) return tree;
    if (Array.isArray(tree) && tree.length > 0) return `#${base58(tree.map((byte) => Number(byte) & 0xff))}`;
    return null;
  };
  const revisionBadge = (revision) => {
    const full = revision ? treeDisplay(revision.tree) : null;
    if (!full) return '<wa-badge variant="neutral" appearance="filled">no commits</wa-badge>';
    const short = (full.startsWith('#') ? full.slice(1) : full).slice(0, 8);
    return `<wa-badge variant="neutral" appearance="filled" title="${esc(full)}">`
      + `<wa-icon name="code-commit" slot="start"></wa-icon>${esc(short)}</wa-badge>`;
  };

  const fieldValue = (value) => {
    let kind = 'tonk-cm-plain';
    let text;
    if (value === null || value === undefined) { kind = 'tonk-cm-variable'; text = '_'; }
    else if (typeof value === 'boolean' || typeof value === 'number') { kind = 'tonk-cm-number'; text = String(value); }
    else if (typeof value === 'string') {
      if (looksLikeUri(value)) { kind = 'tonk-cm-entity'; text = spellUri(value); }
      else { kind = 'tonk-cm-string'; text = value; }
    } else text = JSON.stringify(value);
    return `<span class="${kind}">${esc(text)}</span>`;
  };

  // Listed (notation) view.
  const indent = (depth) => '  '.repeat(depth);
  const keyRow = (pad, name, dash) =>
    `<div class="notation-row notation-field"><span class="notation-indent">${esc(pad)}</span>`
    + (dash ? '<span class="tonk-cm-plain">- </span>' : '')
    + `<span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">:</span></div>`;
  const field = (depth, name, value) => {
    const pad = indent(depth);
    if (isObject(value)) {
      return keyRow(pad, name, false) + Object.entries(value).map(([k, v]) => field(depth + 1, k, v)).join('');
    }
    if (Array.isArray(value)) {
      const dashPad = indent(depth + 1);
      let out = keyRow(pad, name, false);
      for (const item of value) {
        if (isObject(item)) {
          const entries = Object.entries(item);
          if (entries.length > 0) out += dashField(dashPad, depth + 2, entries[0][0], entries[0][1]);
          for (const [k, v] of entries.slice(1)) out += field(depth + 2, k, v);
        } else {
          out += `<div class="notation-row notation-field"><span class="notation-indent">${esc(dashPad)}</span>`
            + `<span class="tonk-cm-plain">- </span>${fieldValue(item)}</div>`;
        }
      }
      return out;
    }
    if (typeof value === 'string' && value.includes('\n')) {
      const linePad = indent(depth + 1);
      return keyRow(pad, name, false) + value.split('\n').map((line) =>
        `<div class="notation-row notation-value-line"><span class="notation-indent">${esc(linePad)}</span>`
        + `<span class="tonk-cm-string">${esc(line)}</span></div>`).join('');
    }
    return `<div class="notation-row notation-field"><span class="notation-indent">${esc(pad)}</span>`
      + `<span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">: </span>${fieldValue(value)}</div>`;
  };
  // The first field of an object item in a sequence shares the `- ` row.
  const dashField = (dashPad, childDepth, name, value) => {
    if (isObject(value)) {
      return keyRow(dashPad, name, true) + Object.entries(value).map(([k, v]) => field(childDepth + 1, k, v)).join('');
    }
    if (Array.isArray(value)) return keyRow(dashPad, name, true) + field(childDepth, '', value);
    return `<div class="notation-row notation-field"><span class="notation-indent">${esc(dashPad)}</span>`
      + `<span class="tonk-cm-plain">- </span><span class="tonk-cm-key">${esc(name)}</span>`
      + `<span class="tonk-cm-plain">: </span>${fieldValue(value)}</div>`;
  };
  const record = (head, body) =>
    `<div class="notation-record"><div class="notation-row"><span class="tonk-cm-effect">${esc(head)}!:</span></div>${body}</div>`;
  const notationRecord = (label, result) => {
    if (label === 'concept' || label === 'command') {
      const map = descriptor(result, label === 'concept');
      return record(label, field(1, 'this', result.this) + Object.entries(map ?? {}).map(([k, v]) => field(1, k, v)).join(''));
    }
    if (label === 'rule') {
      const map = ruleDefinition(result);
      return record('rule', field(1, 'this', result.this) + Object.entries(map ?? {}).map(([k, v]) => field(1, k, v)).join(''));
    }
    // Collection entries arrive folded (`show: {ui: …}`), so they nest.
    return record(label, field(1, 'this', result.this)
      + sorted(result.fields ?? {}).filter(([name]) => name !== 'this').map(([k, v]) => field(1, k, v)).join(''));
  };
  const notation = (blocks) =>
    `<div class="query-notation wa-stack wa-gap-s">${blocks
      .flatMap((block) => (block.results ?? []).map((result) => notationRecord(block.label, result)))
      .join('')}</div>`;

  // Grouped tree view.
  const treeValue = (name, value) => {
    if (isObject(value)) {
      return `<wa-tree-item expanded><span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">:</span>`
        + Object.entries(value).map(([k, v]) => treeValue(k, v)).join('') + '</wa-tree-item>';
    }
    if (Array.isArray(value)) {
      const items = value.map((item) => (isObject(item)
        ? `<wa-tree-item expanded><span class="tonk-cm-plain">-</span>${Object.entries(item).map(([k, v]) => treeValue(k, v)).join('')}</wa-tree-item>`
        : `<wa-tree-item><span class="tonk-cm-plain">- </span>${fieldValue(item)}</wa-tree-item>`)).join('');
      return `<wa-tree-item expanded><span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">:</span>${items}</wa-tree-item>`;
    }
    return `<wa-tree-item expanded><span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">:</span>`
      + `<wa-tree-item>${fieldValue(value)}</wa-tree-item></wa-tree-item>`;
  };
  const treeItem = (label, result) => {
    if (label === 'concept' || label === 'command' || label === 'rule') {
      const map = label === 'rule' ? ruleDefinition(result) : descriptor(result, label === 'concept');
      const head = label === 'rule'
        ? '<span class="tonk-cm-effect">rule!</span><span class="tonk-cm-plain">:</span>'
        : `<span class="tonk-cm-effect">${esc(label)}</span><span class="tonk-cm-plain">!:</span>`;
      return `<wa-tree-item expanded>${head}${treeValue('this', result.this)}`
        + Object.entries(map ?? {}).map(([k, v]) => treeValue(k, v)).join('') + '</wa-tree-item>';
    }
    const fields = sorted(result.fields ?? {}).map(([name, value]) =>
      `<wa-tree-item expanded><span class="tonk-cm-key">${esc(name)}</span><span class="tonk-cm-plain">:</span>`
      + `<wa-tree-item>${fieldValue(value)}</wa-tree-item></wa-tree-item>`).join('');
    return `<wa-tree-item expanded><span class="tonk-cm-entity">${esc(result.this)}</span><span class="tonk-cm-plain">:</span>${fields}</wa-tree-item>`;
  };
  const tree = (blocks) => `<wa-tree class="query-tree">${blocks.map((block) =>
    `<wa-tree-item expanded><span class="tonk-cm-effect">${esc(block.label)}</span><span class="tonk-cm-plain">:</span>`
    + (block.results ?? []).map((result) => treeItem(block.label, result)).join('') + '</wa-tree-item>').join('')}</wa-tree>`;

  // Table view: one table per query, a column per field in first-seen order.
  const table = (block) => {
    const columns = [];
    for (const result of block.results ?? []) {
      for (const name of Object.keys(result.fields ?? {}).sort()) {
        if (name !== 'this' && !columns.includes(name)) columns.push(name);
      }
    }
    const head = columns.map((name) => `<th>${esc(name)}</th>`).join('');
    const rows = (block.results ?? []).map((result) => {
      const cells = columns.map((name) => {
        const has = result.fields && Object.prototype.hasOwnProperty.call(result.fields, name);
        return `<td>${has ? `<span>${fieldValue(result.fields[name])}</span>` : ''}</td>`;
      }).join('');
      return `<tr><td class="query-table-this"><wa-copy-button value="${esc(result.this)}"><span>${esc(result.this)}</span></wa-copy-button></td>${cells}</tr>`;
    }).join('');
    return `<div class="query-table"><table><thead><tr><th class="query-table-this"><span>${esc(block.label)}</span></th>${head}</tr></thead><tbody>${rows}</tbody></table></div>`;
  };
  const tables = (blocks) => `<div class="query-tables wa-stack wa-gap-l">${blocks.map(table).join('')}</div>`;

  const tabs = (blocks) =>
    '<wa-tab-group class="evaluate-tabs" placement="end">'
    + '<wa-tab panel="listed"><wa-icon name="list" variant="solid"></wa-icon></wa-tab>'
    + '<wa-tab panel="tree"><wa-icon name="folder-tree" variant="solid"></wa-icon></wa-tab>'
    + '<wa-tab panel="table"><wa-icon name="table" variant="solid"></wa-icon></wa-tab>'
    + `<wa-tab-panel name="listed">${notation(blocks)}</wa-tab-panel>`
    + `<wa-tab-panel name="tree">${tree(blocks)}</wa-tab-panel>`
    + `<wa-tab-panel name="table">${tables(blocks)}</wa-tab-panel>`
    + '</wa-tab-group>';

  const matches = () => {
    const before = response.matches_before ?? [];
    const after = response.matches_after ?? [];
    const latest = response.revision_after ?? response.revision_before;
    if (before.length === 0 && after.length === 0) {
      return `<div class="evaluate-revision">${revisionBadge(latest)}</div>`;
    }
    if (JSON.stringify(before) === JSON.stringify(after)) {
      return `<div class="evaluate-results wa-stack wa-gap-2xs"><div class="evaluate-revision">${revisionBadge(latest)}</div>${tabs(after)}</div>`;
    }
    return '<wa-comparison position="50" class="evaluate-comparison">'
      + '<div slot="before" class="evaluate-side evaluate-side-before wa-stack wa-gap-2xs">'
      + `<div class="evaluate-revision">${revisionBadge(response.revision_before)}</div>${notation(before)}</div>`
      + '<div slot="after" class="evaluate-side evaluate-side-after wa-stack wa-gap-2xs">'
      + `<div class="evaluate-revision">${revisionBadge(response.revision_after)}</div>${notation(after)}</div>`
      + '</wa-comparison>';
  };

  const callout = failure == null ? ''
    : `<wa-callout variant="danger"><wa-icon slot="icon" name="circle-exclamation"></wa-icon>${esc(failure)}</wa-callout>`;
  const content = response ? matches() : '';
  return `<div class="evaluate-result"><div class="evaluate-failure">${callout}</div><div class="evaluate-content">${content}</div></div>`;
}
