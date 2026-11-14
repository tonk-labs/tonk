// A cell's output as HTML: a failure, or what an evaluate `response`
// matched, or nothing.
//
// Compact, because it sits between two paragraphs: a summary line per
// query ("3 results"), then a gallery of cards, the first twelve shown and
// the rest folded behind one line. A result whose model has a view is a
// card drawn by a <tonk-display> of it (the model is the result's own
// `model` field, else the query's label), headed by its name and entity. A
// command's result, never stored, and a schema row are drawn as the
// notation that would assert them, which can be copied into another cell.
//
// `context` is the notebook's `branch@repo`, stamped on every display: the
// cards render inside the editor's shadow root, where a display reads its
// own `with` and nothing above it.
(self, failure, response, context) => {
  const CARD_CAP = 12;
  const esc = (text) => String(text)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

  if (failure !== null && failure !== undefined) {
    return `<div class="nb-out nb-out--error"><span class="nb-out__icon">!</span><span class="nb-out__text">${esc(failure)}</span></div>`;
  }
  if (!response) return '';

  const scheme = (text) => /^[a-z][a-z0-9+-]*$/.test(text);
  // A bare-scheme URI (`keyword:`) is spelled `:keyword`.
  const spellUri = (uri) => (uri.endsWith(':') && scheme(uri.slice(0, -1)) ? `:${uri.slice(0, -1)}` : uri);
  const looksLikeUri = (text) => !/\s/.test(text) && (text.includes(':') || (text.includes('.') && text.includes('/')));

  // A stored type as notation types it, by its anchor (`text`, `natural:64`).
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

  // A result's own fields, sorted, minus `this`, which the head names.
  const plainFields = (result) => Object.keys(result.fields ?? {}).sort()
    .filter((name) => name !== 'this')
    .map((name) => [name, result.fields[name]]);

  const writeKey = (key) => {
    const bare = key !== '' && !/^[-?]/.test(key) && /^[\p{L}\p{N}_\-./!?]+$/u.test(key);
    return `${bare ? key : JSON.stringify(key)}:`;
  };
  const scalar = (key, value) => {
    if (value === null || value === undefined) return '_';
    if (typeof value === 'boolean' || typeof value === 'number') return String(value);
    if (Array.isArray(value)) return '[]';
    if (isObject(value)) return '{}';
    const text = String(value);
    const anchor = key === 'as' && text !== '' && /^[a-z0-9:/-]+$/.test(text);
    if (anchor || (text.startsWith('?') && !/\s/.test(text))) return text;
    if (looksLikeUri(text)) return spellUri(text);
    return JSON.stringify(text);
  };
  const afterKey = (key, value, indent) => {
    if (isObject(value) && Object.keys(value).length) {
      return '\n' + Object.entries(value).map(([name, child]) => entry(name, child, indent + 2)).join('');
    }
    if (Array.isArray(value) && value.length) {
      let out = '\n';
      for (const item of value) {
        out += `${' '.repeat(indent + 2)}-`;
        if (isObject(item) && Object.keys(item).length) {
          Object.entries(item).forEach(([name, child], index) => {
            out += index === 0
              ? ` ${writeKey(name)}${afterKey(name, child, indent + 4)}`
              : entry(name, child, indent + 4);
          });
        } else {
          out += afterKey(key, item, indent + 2);
        }
      }
      return out;
    }
    // Text with a line break or a `"` is a literal block, unescaped.
    if (typeof value === 'string' && (value.includes('\n') || (value.includes('"') && !looksLikeUri(value)))) {
      const [chomp, body] = value.endsWith('\n') ? ['|', value.slice(0, -1)] : ['|-', value];
      return ` ${chomp}\n` + body.split('\n').map((line) => `${line ? ' '.repeat(indent + 2) + line : ''}\n`).join('');
    }
    return ` ${scalar(key, value)}\n`;
  };
  const entry = (key, value, indent) => `${' '.repeat(indent)}${writeKey(key)}${afterKey(key, value, indent)}`;

  // A result as the notation that asserts it; a schema row as its
  // definition, rebuilt from the stored descriptor.
  const notation = (result, label) => {
    const head = label.trim();
    let fields = null;
    if (head === 'concept' || head === 'command') {
      const map = descriptor(result, head === 'concept');
      fields = map && Object.entries(map);
    } else if (head === 'rule') {
      const map = ruleDefinition(result);
      fields = map && Object.entries(map);
    }
    fields ??= plainFields(result);
    let out = `${head || '_'}!:\n  this: ${spellUri(result.this)}\n`;
    for (const [name, value] of fields) out += entry(name, value, 2);
    return out.trimEnd();
  };

  // The model a label names, unless it is a meta head, whose rows are
  // schema rather than data a view could draw.
  const modelFromLabel = (label) => {
    const trimmed = label.trim();
    return !trimmed || ['concept', 'attribute', 'command', 'rule', 'name', 'view'].includes(trimmed) ? null : trimmed;
  };
  const modelOf = (fields) => (typeof fields?.model === 'string' && fields.model !== '' ? fields.model : null);

  const card = (result, label) => {
    // A command's result is in the response but was never written, so a
    // display re-reading it would find nothing: drawn from its fields.
    const model = result.transient ? null : (modelOf(result.fields) ?? modelFromLabel(label));
    if (model) {
      const name = typeof result.fields?.name === 'string' ? result.fields.name.trim() : '';
      const named = name ? `<span class="nb-card__name">${esc(name)}</span>` : '';
      return `<div class="nb-card nb-card--display"><div class="nb-card__title">${named}`
        + `<span class="nb-card__entity">${esc(spellUri(result.this))}</span></div>`
        + `<tonk-display entity="${esc(result.this)}" model="${esc(model)}" with="${esc(context)}"></tonk-display></div>`;
    }
    const text = notation(result, label).replace(/<\/script/g, '<\\/script');
    return `<div class="nb-card nb-card--notation"><tonk-notation><script type="text/tonk-notation">${text}</script></tonk-notation></div>`;
  };

  const block = ({ label, results = [] }) => {
    const count = results.length;
    const summary = `<div class="nb-out__summary"><span class="nb-out__label">${esc(label)}</span>`
      + `<span class="nb-out__count">${count === 0 ? 'no results' : count === 1 ? '1 result' : `${count} results`}</span></div>`;
    if (count === 0) return `<div class="nb-out">${summary}</div>`;
    const cards = results.map((result) => card(result, label));
    const shown = cards.slice(0, CARD_CAP);
    const rest = cards.slice(CARD_CAP);
    const more = rest.length
      ? `<details class="nb-out__rest"><summary class="nb-out__more">${rest.length} more</summary>`
        + `<div class="nb-out__gallery">${rest.join('')}</div></details>`
      : '';
    return `<div class="nb-out">${summary}<div class="nb-out__gallery">${shown.join('')}</div>${more}</div>`;
  };

  // The blocks a commit produced, else the ones it read.
  const after = response.matches_after ?? [];
  const blocks = after.length ? after : (response.matches_before ?? []);
  return blocks.map(block).join('');
}
