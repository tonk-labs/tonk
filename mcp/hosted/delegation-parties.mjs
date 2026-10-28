// Reads who issued a delegation, and to whom, from the hex `DelegationChain`
// the Tonk approval page returns. It does NOT verify the signature: callers may
// use the parties only for display and routing, never as proof of authority.

const MAX_DEPTH = 16;

// Minimal deterministic CBOR (dag-cbor) reader: the major types a UCAN
// container uses. Tags are unwrapped; floats are refused.
function decode(bytes) {
  let offset = 0;
  const take = (count) => {
    if (offset + count > bytes.length) throw Error('Truncated CBOR');
    const slice = bytes.subarray(offset, offset + count);
    offset += count;
    return slice;
  };
  const length = (info) => {
    if (info < 24) return info;
    const size = { 24: 1, 25: 2, 26: 4, 27: 8 }[info];
    if (!size) throw Error('Unsupported CBOR length');
    let value = 0;
    for (const byte of take(size)) value = value * 256 + byte;
    if (!Number.isSafeInteger(value)) throw Error('CBOR length out of range');
    return value;
  };
  const item = (depth) => {
    if (depth > MAX_DEPTH) throw Error('CBOR nested too deeply');
    const head = take(1)[0];
    const major = head >> 5,
      info = head & 31;
    switch (major) {
      case 0:
        return length(info);
      case 1:
        return -1 - length(info);
      case 2:
        return take(length(info));
      case 3:
        return new TextDecoder('utf-8', { fatal: true }).decode(take(length(info)));
      case 4: {
        const count = length(info);
        const out = [];
        for (let i = 0; i < count; i++) out.push(item(depth + 1));
        return out;
      }
      case 5: {
        const count = length(info);
        const out = new Map();
        for (let i = 0; i < count; i++) out.set(item(depth + 1), item(depth + 1));
        return out;
      }
      case 6:
        length(info);
        return item(depth + 1);
      case 7:
        if (info === 20) return false;
        if (info === 21) return true;
        if (info === 22) return null;
        throw Error('Unsupported CBOR simple value');
    }
  };
  const value = item(0);
  if (offset !== bytes.length) throw Error('Trailing CBOR bytes');
  return value;
}

const DELEGATION_TAG = /^ucan\/dlg@/;
const DID_KEY = /^did:key:z[1-9A-HJ-NP-Za-km-z]{40,60}$/;

// `{ issuer, audience }` of the single delegation in a one-link chain.
export function delegationParties(hex) {
  if (typeof hex !== 'string' || !/^(?:[0-9a-f]{2})+$/.test(hex) || hex.length > 64_000)
    throw Error('Invalid delegation hex');
  const container = decode(Buffer.from(hex, 'hex'));
  const tokens = container instanceof Map ? container.get('ctn-v1') : undefined;
  if (!Array.isArray(tokens) || tokens.length !== 1 || !(tokens[0] instanceof Uint8Array))
    throw Error('Expected a container of exactly one delegation');
  const envelope = decode(tokens[0]);
  const signed = Array.isArray(envelope) && envelope.length === 2 ? envelope[1] : undefined;
  if (!(signed instanceof Map)) throw Error('Invalid delegation envelope');
  const tags = [...signed.keys()].filter(
    (key) => typeof key === 'string' && DELEGATION_TAG.test(key),
  );
  if (tags.length !== 1) throw Error('Expected one delegation payload');
  const payload = signed.get(tags[0]);
  const issuer = payload instanceof Map ? payload.get('iss') : undefined;
  const audience = payload instanceof Map ? payload.get('aud') : undefined;
  if (!DID_KEY.test(issuer ?? '') || !DID_KEY.test(audience ?? ''))
    throw Error('Delegation parties must be did:key');
  return { issuer, audience };
}
