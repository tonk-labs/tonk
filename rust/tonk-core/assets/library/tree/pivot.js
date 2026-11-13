// The routing pivot of the raw key `hex`: the index of the last byte that
// must stay bright for it to tell apart from both its neighbours, the
// greater of where it diverges from `prev` (the previous sibling's key, or
// all zeroes for a first child) and from `next`. Its length when it
// diverges from neither, and null when a key is not hex.
(self, hex, prev, next) => {
  const decode = (key) => {
    const raw = key.startsWith('0x') ? key.slice(2) : key;
    if (raw.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(raw)) return null;
    const bytes = [];
    for (let i = 0; i < raw.length; i += 2) bytes.push(parseInt(raw.slice(i, i + 2), 16));
    return bytes;
  };
  const key = decode(hex);
  if (!key) return null;
  const diverge = (other) => {
    const n = Math.min(key.length, other.length);
    for (let i = 0; i < n; i++) if (key[i] !== other[i]) return i;
    return null;
  };
  const lower = prev ? decode(prev) : new Array(key.length).fill(0);
  if (!lower) return null;
  const fromPrev = diverge(lower);
  const upper = next ? decode(next) : null;
  const fromNext = upper ? diverge(upper) : null;
  if (fromPrev !== null && fromNext !== null) return Math.max(fromPrev, fromNext);
  return fromPrev ?? fromNext ?? key.length;
}
