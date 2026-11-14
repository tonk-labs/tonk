// A fact's value spelled so its type reads from the text: a string
// quoted, a float always with a `.`, bytes as hex pairs, a number plain.
// `type` is the value's dialog type name (`text:`, `natural:64`, or the
// name an earlier release wrote, `Text`); a sized number type formats as
// the numbers it stores.
(self, value, type) => {
  const sized = String(type ?? '').match(/^(natural|integer|float):(\d+)$/);
  const kind = sized ? `${sized[1]}:` : type;
  const json = (v) => JSON.stringify(v);
  switch (kind) {
    case 'Text':
    case 'text:':
      return typeof value === 'string' ? `"${value}"` : json(value);
    case 'Float':
    case 'float:':
      if (typeof value === 'number') return Number.isInteger(value) ? value.toFixed(1) : String(value);
      return json(value);
    case 'SignedInt':
    case 'SignedInteger':
    case 'integer:':
      if (typeof value === 'number') return String(Math.trunc(value));
      return typeof value === 'string' ? value : json(value);
    case 'Bytes':
    case 'bytes:':
      if (typeof value === 'string') return value;
      if (Array.isArray(value)) {
        return value
          .filter((byte) => Number.isInteger(byte) && byte >= 0)
          .map((byte) => byte.toString(16).padStart(2, '0'))
          .join(' ');
      }
      return json(value);
    default:
      return typeof value === 'string' ? value : json(value);
  }
}
