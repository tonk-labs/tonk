// Positions: the keys of a notebook's `block` sequence.
//
// A position is dialog's fractional position (`dialog_artifacts::position`,
// the same derivation the `dialog/position` formula runs): an ASCII string
// over `0-9A-Za-z` that sorts in byte order, a major byte (`A-Z`) naming
// the width of the integer part, then the integer part, then a fractional
// tail. A new position takes its tail from a bias, the leading six base62
// digits of the BLAKE3 digest of the member's entity, so the same block
// moved between the same neighbours lands on the same key on every
// replica, and on the key the rules would derive for it.
//
// What a commit needs from it is `moves`: the surviving blocks that need a
// new key, with the key. The longest run of keys already in order is kept,
// so a block moved across its neighbours is the one re-keyed.
//
// Pure: no DOM, no state. Returns the functions, evaluated once per tag.
(self) => (self.constructor.__notebookPositions ??= (() => {
  // BLAKE3, the whole tree, for the 32-byte digest only.
  const IV = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
  const PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
  const CHUNK_START = 1;
  const CHUNK_END = 2;
  const PARENT = 4;
  const ROOT = 8;
  const rotr = (word, by) => ((word >>> by) | (word << (32 - by))) >>> 0;
  const g = (s, a, b, c, d, x, y) => {
    s[a] = (s[a] + s[b] + x) >>> 0;
    s[d] = rotr(s[d] ^ s[a], 16);
    s[c] = (s[c] + s[d]) >>> 0;
    s[b] = rotr(s[b] ^ s[c], 12);
    s[a] = (s[a] + s[b] + y) >>> 0;
    s[d] = rotr(s[d] ^ s[a], 8);
    s[c] = (s[c] + s[d]) >>> 0;
    s[b] = rotr(s[b] ^ s[c], 7);
  };
  const compress = (cv, words, counter, length, flags) => {
    const s = [...cv, IV[0], IV[1], IV[2], IV[3], counter >>> 0, Math.floor(counter / 0x100000000) >>> 0, length, flags];
    let m = words.slice();
    for (let round = 0; round < 7; round++) {
      g(s, 0, 4, 8, 12, m[0], m[1]);
      g(s, 1, 5, 9, 13, m[2], m[3]);
      g(s, 2, 6, 10, 14, m[4], m[5]);
      g(s, 3, 7, 11, 15, m[6], m[7]);
      g(s, 0, 5, 10, 15, m[8], m[9]);
      g(s, 1, 6, 11, 12, m[10], m[11]);
      g(s, 2, 7, 8, 13, m[12], m[13]);
      g(s, 3, 4, 9, 14, m[14], m[15]);
      m = PERMUTATION.map((index) => m[index]);
    }
    for (let i = 0; i < 8; i++) s[i] = (s[i] ^ s[i + 8]) >>> 0;
    return s.slice(0, 8);
  };
  const block = (bytes, from, to) => {
    const words = new Array(16).fill(0);
    for (let i = from; i < to; i++) words[(i - from) >> 2] |= bytes[i] << (8 * ((i - from) & 3));
    return words.map((word) => word >>> 0);
  };
  const chunk = (bytes, start, end, counter, root) => {
    let cv = IV;
    const last = Math.max(start, end - 1 - ((end - start - 1) % 64 + 64) % 64);
    for (let at = start; ; at += 64) {
      const to = Math.min(at + 64, end);
      const final = at >= last;
      const flags = (at === start ? CHUNK_START : 0) | (final ? CHUNK_END | (root ? ROOT : 0) : 0);
      cv = compress(cv, block(bytes, at, to), counter, to - at, flags);
      if (final) return cv;
    }
  };
  const subtree = (bytes, first, count, root) => {
    if (count === 1) return chunk(bytes, first * 1024, Math.min((first + 1) * 1024, bytes.length), first, root);
    let left = 1;
    while (left * 2 < count) left *= 2;
    const words = [...subtree(bytes, first, left, false), ...subtree(bytes, first + left, count - left, false)];
    return compress(IV, words, 0, 64, PARENT | (root ? ROOT : 0));
  };
  const blake3 = (bytes) => {
    const words = subtree(bytes, 0, Math.max(1, Math.ceil(bytes.length / 1024)), true);
    return words.flatMap((word) => [word & 0xff, (word >>> 8) & 0xff, (word >>> 16) & 0xff, word >>> 24]);
  };

  // Digits, as byte values.
  const code = (ch) => ch.charCodeAt(0);
  const B62 = [[code('0'), code('9')], [code('A'), code('Z')], [code('a'), code('z')]];
  const MAJORS = [[code('A'), code('M')], [code('N'), code('Z')]];
  const MIN = code('0');
  const MAX = code('z');
  const MEDIAN = code('V');
  const ZERO = code('N');
  const BIAS_DIGITS = 6;
  const CODES = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz';
  const inRanges = (byte, ranges) => ranges.some(([from, to]) => byte >= from && byte <= to);
  const less = (a, b) => {
    for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] < b[i];
    return a.length < b.length;
  };
  const bytesOf = (text) => Array.from(text, code);
  const textOf = (bytes) => String.fromCharCode(...bytes);

  // Base62, most significant digit first, leading zero bytes kept.
  const base62 = (bytes) => {
    let zeros = 0;
    while (zeros < bytes.length && bytes[zeros] === 0) zeros += 1;
    const encoding = [];
    for (const byte of bytes.slice(zeros)) {
      let carry = byte;
      for (let i = 0; carry !== 0 || i < encoding.length; i++) {
        if (i === encoding.length) encoding.push(0);
        carry += encoding[i] << 8;
        encoding[i] = carry % 62;
        carry = Math.floor(carry / 62);
      }
    }
    return [...new Array(zeros).fill(MIN), ...encoding.reverse().map((digit) => code(CODES[digit]))];
  };
  // The bias a member contributes: six digits of its entity's digest.
  const bias = (entity) => base62(blake3(Array.from(new TextEncoder().encode(entity)))).slice(0, BIAS_DIGITS);

  // A canonical position: every byte in the alphabet, a major first, no
  // trailing minimum digit. Null when `text` is none.
  const parse = (text) => {
    if (typeof text !== 'string' || text === '') return null;
    const bytes = bytesOf(text);
    if (!inRanges(bytes[0], MAJORS)) return null;
    if (bytes.slice(1).some((byte) => !inRanges(byte, B62))) return null;
    if (bytes[bytes.length - 1] === MIN) return null;
    return bytes;
  };

  const capacity = (major) => {
    const [[outerNegative, innerNegative], [innerPositive, outerPositive]] = MAJORS;
    if (major >= innerPositive && major <= outerPositive) return major - innerPositive + 1;
    if (major >= outerNegative && major <= innerNegative) return innerNegative - major + 1;
    return 0;
  };
  const minorOf = (position) => {
    const width = capacity(position[0]);
    const stored = position.slice(1, Math.min(1 + width, position.length));
    return [...stored, ...new Array(width - stored.length).fill(MIN)];
  };
  const patchOf = (position) => position.slice(1 + capacity(position[0]));
  const trim = (digits) => {
    let length = digits.length;
    while (length > 0 && digits[length - 1] === MIN) length -= 1;
    return digits.slice(0, length);
  };
  const create = (major, minor, patch) => {
    const tail = trim(patch);
    const width = capacity(major);
    if (!tail.length) return [major, ...trim(minor)];
    const kept = minor.slice(0, Math.min(minor.length, width));
    return [major, ...kept, ...new Array(width - kept.length).fill(MIN), ...tail];
  };

  const digitIncrement = (digit, ranges) => {
    const next = digit + 1;
    for (const [from, to] of ranges) {
      if (next < from) return from;
      if (next <= to) return next;
    }
    return null;
  };
  const digitDecrement = (digit, ranges) => {
    const previous = digit - 1;
    if (previous < 0) return null;
    for (const [from, to] of [...ranges].reverse()) {
      if (previous > to) return to;
      if (previous >= from) return previous;
    }
    return null;
  };
  // A digit strictly between, or 'equal' / 'consecutive'.
  const digitIntermediate = (from, to, ranges) => {
    const [bottom, top] = from > to ? [to, from] : [from, to];
    if (bottom === top) return 'equal';
    if (top - bottom === 1) return 'consecutive';
    const digit = Math.ceil((bottom + top) / 2);
    let last = null;
    for (const [low, high] of ranges) {
      if (digit < low) {
        if (bottom < low && low < top) return low;
        if (last !== null && bottom < last && last < high) return last;
        return 'consecutive';
      }
      if (low <= digit && digit <= high) return digit;
      last = high;
    }
    if (last !== null && bottom < last && last < top) return last;
    return 'consecutive';
  };
  const digitsIncrement = (source, ranges) => {
    const digits = source.slice();
    for (let offset = digits.length - 1; offset >= 0; offset--) {
      const digit = digitIncrement(digits[offset], ranges);
      if (digit === null) {
        digits[offset] = ranges[0][0];
      } else {
        digits[offset] = digit;
        return digits.slice(0, offset + 1);
      }
    }
    return null;
  };
  const digitsDecrement = (source, ranges) => {
    const digits = source.slice();
    const max = ranges[ranges.length - 1][1];
    for (let offset = digits.length - 1; offset >= 0; offset--) {
      const digit = digitDecrement(digits[offset], ranges);
      if (digit === null) {
        digits[offset] = max;
      } else {
        digits[offset] = digit;
        return trim(digits);
      }
    }
    return null;
  };
  // Digits strictly between `begin` and `end` (null: the open upper
  // bound), or 'equal' / 'consecutive'. Missing digits read as the
  // minimum, missing upper digits as one past the maximum.
  const digitsIntermediate = (begin, end, ranges) => {
    const min = ranges[0][0];
    const max = ranges[ranges.length - 1][1];
    const length = Math.max(begin.length, end ? end.length : 0, end ? 0 : 1);
    const digits = new Array(length).fill(0);
    let offset = 0;
    if (end) {
      while (offset < length) {
        const lower = begin[offset] ?? min;
        const upper = end[offset] ?? min;
        if (lower !== upper) break;
        digits[offset] = lower;
        offset += 1;
      }
      if (offset === length) return 'equal';
    }
    const reversed = end ? (begin[offset] ?? min) > (end[offset] ?? min) : false;
    const [from, to] = reversed ? [end, begin] : [begin, end];
    let bounded = true;
    while (offset < length) {
      const low = from[offset] ?? min;
      const high = bounded ? (to?.[offset] ?? max + 1) : max + 1;
      const digit = digitIntermediate(low, high, ranges);
      if (digit === 'equal' || digit === 'consecutive') {
        bounded = bounded && low === high;
        digits[offset] = low;
        offset += 1;
      } else {
        digits[offset] = digit;
        return digits.slice(0, offset + 1);
      }
    }
    return 'consecutive';
  };

  const patchIntermediate = (low, high, tail) => {
    const digits = digitsIntermediate(low, high, B62);
    if (digits === 'equal') return null;
    if (digits === 'consecutive') return [...low, ...(tail.length ? tail : [MEDIAN])];
    if (!tail.length) return digits;
    const head = tail[0];
    const offset = digits.length - 1;
    let ceiling = null;
    if (high) {
      let shared = true;
      for (let index = 0; index < offset; index++) if (digits[index] !== (high[index] ?? MIN)) shared = false;
      ceiling = shared ? (high[offset] ?? null) : null;
    }
    const floor = low[offset];
    const fits = floor !== undefined && ceiling !== null && floor < head && head < ceiling;
    return [...digits.slice(0, fits ? offset : offset + 1), ...tail];
  };
  const patchNext = (digits, tail) => patchIntermediate(digits, null, tail) ?? [...digits, ...tail];
  const patchDecrease = (digits) => {
    let offset = digits.length - 1;
    if (offset < 0) return null;
    while (offset > 0 && digits[offset] === MIN) offset -= 1;
    return offset === 0 ? null : digits.slice(0, offset + 1);
  };
  const patchIncrement = (patch, tail) => {
    const digits = digitsIncrement(patch, B62);
    if (digits === null) return [...patch, MEDIAN];
    if (!tail.length) return digits;
    return !less(tail, digits) ? tail.slice() : [...digits, ...tail];
  };
  const patchDecrement = (patch, tail) => {
    const digits = digitsDecrement(patch, B62) ?? patchDecrease(patch);
    if (digits === null) return null;
    if (!tail.length) return digits;
    const candidate = !less(digits, tail) ? tail.slice() : [...digits, ...tail];
    return less(candidate, patch) ? candidate : digits;
  };

  const afterPosition = (tail, position) => {
    const major = position[0];
    const minor = minorOf(position);
    const stepped = digitsIncrement(minor, B62);
    if (stepped) return create(major, stepped, tail);
    const next = digitIncrement(major, MAJORS);
    if (next !== null) return create(next, new Array(capacity(next)).fill(MIN), tail);
    return create(major, minor, patchIncrement(patchOf(position), tail));
  };
  const beforePosition = (tail, position) => {
    const major = position[0];
    const minor = minorOf(position);
    const stepped = digitsDecrement(minor, B62);
    if (stepped) return create(major, stepped, tail);
    const previous = digitDecrement(major, MAJORS);
    if (previous !== null) return create(previous, new Array(capacity(previous)).fill(MAX), tail);
    const patch = patchDecrement(patchOf(position), tail);
    return patch === null ? null : create(major, minor, patch);
  };
  const between = (tail, low, high) => {
    const lowMajor = low[0];
    const highMajor = high[0];
    const major = digitIntermediate(lowMajor, highMajor, MAJORS);
    if (major === 'equal') {
      const lowMinor = minorOf(low);
      const highMinor = minorOf(high);
      const minor = digitsIntermediate(lowMinor, highMinor, B62);
      if (minor === 'equal') {
        const patch = patchIntermediate(patchOf(low), patchOf(high), tail);
        return patch === null ? low.slice() : create(lowMajor, lowMinor, patch);
      }
      if (minor === 'consecutive') {
        if (patchDecrement(patchOf(high), tail) !== null) return create(highMajor, highMinor, []);
        return create(lowMajor, lowMinor, patchNext(patchOf(low), tail));
      }
      return create(lowMajor, minor, tail);
    }
    if (major === 'consecutive') {
      const up = digitsIncrement(minorOf(low), B62);
      if (up) return create(lowMajor, up, tail);
      const down = digitsDecrement(minorOf(high), B62);
      if (down) return create(highMajor, down, tail);
      if (patchDecrement(patchOf(high), tail) !== null) return create(highMajor, minorOf(high), []);
      return create(lowMajor, minorOf(low), patchNext(patchOf(low), tail));
    }
    return create(major, new Array(capacity(major)).fill(MIN), tail);
  };

  // The position for `entity` between the positions `low` and `high`
  // (null: open), or null when no position exists there (before the
  // absolute minimum) or a bound is not a position.
  const insert = (entity, low, high) => {
    const tail = bias(entity);
    const lower = low === null ? null : parse(low);
    const upper = high === null ? null : parse(high);
    if ((low !== null && !lower) || (high !== null && !upper)) return null;
    let out;
    if (lower && upper) out = less(upper, lower) ? between(tail, upper, lower) : between(tail, lower, upper);
    else if (lower) out = afterPosition(tail, lower);
    else if (upper) out = beforePosition(tail, upper);
    else out = create(ZERO, [], tail);
    return out === null ? null : textOf(out);
  };

  // Indices of the longest run of present keys in ascending order.
  const longestIncreasingRun = (keys) => {
    const best = keys.map(() => 0);
    const previous = keys.map(() => -1);
    let end = -1;
    keys.forEach((key, i) => {
      if (key === null) return;
      best[i] = 1;
      for (let j = 0; j < i; j++) {
        if (keys[j] !== null && keys[j] < key && best[j] + 1 > best[i]) {
          best[i] = best[j] + 1;
          previous[i] = j;
        }
      }
      if (end === -1 || best[i] > best[end]) end = i;
    });
    const run = [];
    for (let cursor = end; cursor !== -1; cursor = previous[cursor]) run.push(cursor);
    return run.reverse();
  };

  // Keys for a document's blocks in order (`[[entity, key | null]]`): the
  // longest ascending run keeps its keys, every other block gets one
  // between its nearest kept neighbours. Returns `[[entity, key]]` for the
  // blocks that need one.
  const assignKeys = (order) => {
    const current = order.map(([, key]) => (parse(key) ? key : null));
    const kept = order.map(() => null);
    for (const index of longestIncreasingRun(current)) kept[index] = current[index];
    const placed = [];
    let previous = null;
    order.forEach(([entity], index) => {
      if (kept[index] !== null) {
        previous = kept[index];
        return;
      }
      const next = kept.slice(index + 1).find((key) => key !== null) ?? null;
      const position = insert(entity, previous, next);
      if (position === null) return;
      placed.push([entity, position]);
      previous = position;
    });
    return placed;
  };

  // The surviving blocks of `edit` (from `reconcile`) that need a new key:
  // those that moved, or never had one. Created blocks are placed by the
  // insert rules, so they are not keyed here.
  const moves = (stored, edit) => assignKeys(edit.order
    .filter((entity) => entity !== null)
    .map((entity) => [entity, stored.find((block) => block.entity === entity)?.key ?? null]));

  return Object.freeze({ blake3, bias: (entity) => textOf(bias(entity)), parse: (text) => parse(text) !== null, insert, assignKeys, moves });
})())
