// Decoding and segmenting a composite index key's raw bytes.
//
// A key is the 162 bytes carried as a `#<base58>` string. How those
// bytes split into components depends on the leading tag byte (the
// index ordering): an entity key, an attribute key, and a value key
// slice the same 162 bytes differently. This mirrors dialog-diagnose's
// node inspector, which colors the raw key bytes by tag.

/** base58 alphabet (Bitcoin), matching the `base58` Rust crate. */
const ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const INDEX: Record<string, number> = {};
for (let i = 0; i < ALPHABET.length; i++) INDEX[ALPHABET[i]] = i;

/** Decode a base58 string to bytes. */
export function fromBase58(s: string): Uint8Array {
  const str = s.startsWith("#") ? s.slice(1) : s;
  const bytes: number[] = [];
  for (const ch of str) {
    let carry = INDEX[ch];
    if (carry === undefined) throw new Error(`invalid base58 char ${ch}`);
    for (let j = 0; j < bytes.length; j++) {
      carry += bytes[j] * 58;
      bytes[j] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  // leading '1's are leading zero bytes
  for (const ch of str) {
    if (ch === "1") bytes.push(0);
    else break;
  }
  return new Uint8Array(bytes.reverse());
}

/** One colored run of bytes within a key. */
export interface KeySegment {
  color: string;
  bytes: Uint8Array;
}

/** Index ordering named by a key's tag byte. */
export type KeyTag = "entity" | "attribute" | "value" | "unknown";

// Palette from the index-key talk (hackmd.io/@gozala/HJpfUq-aee):
// Tag pink, Entity teal, Attribute orange, ValueType blue. The `value`
// component reuses the entity teal (it's a value-reference hash).
const COLORS = {
  tag: "#ff8da1",
  entity: "#4ecbc4",
  attribute: "#ffc78e",
  type: "#a1c8ff",
  value: "#4ecbc4",
} as const;

export function tagOf(byte: number): KeyTag {
  return byte === 0 ? "entity" : byte === 1 ? "attribute" : byte === 2 ? "value" : "unknown";
}

/**
 * Slice a 162-byte key into colored segments, tag-driven. The byte
 * layouts mirror dialog-diagnose's NodeInspector:
 *   entity    : [tag 1][entity 64][attribute 64][type 1][value 32]
 *   attribute : [tag 1][attribute 64][entity 64][type 1][value 32]
 *   value     : [tag 1][type 1][value 32][entity 64][attribute 64]
 */
export function segments(key: Uint8Array): KeySegment[] {
  const slice = (a: number, b: number) => key.subarray(a, b);
  switch (tagOf(key[0])) {
    case "entity":
      return [
        { color: COLORS.tag, bytes: slice(0, 1) },
        { color: COLORS.entity, bytes: slice(1, 65) },
        { color: COLORS.attribute, bytes: slice(65, 129) },
        { color: COLORS.type, bytes: slice(129, 130) },
        { color: COLORS.value, bytes: slice(130, 162) },
      ];
    case "attribute":
      return [
        { color: COLORS.tag, bytes: slice(0, 1) },
        { color: COLORS.attribute, bytes: slice(1, 65) },
        { color: COLORS.entity, bytes: slice(65, 129) },
        { color: COLORS.type, bytes: slice(129, 130) },
        { color: COLORS.value, bytes: slice(130, 162) },
      ];
    case "value":
      return [
        { color: COLORS.tag, bytes: slice(0, 1) },
        { color: COLORS.type, bytes: slice(1, 2) },
        { color: COLORS.value, bytes: slice(2, 34) },
        { color: COLORS.entity, bytes: slice(34, 98) },
        { color: COLORS.attribute, bytes: slice(98, 162) },
      ];
    default:
      return [
        { color: "#888", bytes: slice(0, 1) },
        { color: "#555", bytes: slice(1, key.length) },
      ];
  }
}

/** Hex-encode bytes, space-separated. */
export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0").toUpperCase()).join(" ");
}
