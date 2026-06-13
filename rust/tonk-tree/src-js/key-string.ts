// Formatting a composite index key as a pipe-delimited, color-coded,
// tooltipped string — the form used in the tree rows.
//
// A key is the 162 bytes carried as a `#<base58>` string. We slice it
// into its components (tag-driven, since entity/attribute/value index
// keys order the bytes differently), and render each as a short,
// distinguishable string segment:
//
//   1 | did:key:z6Mk…bRD1 | 7J8y…dXq2 | 3 | aF3c…91pQ
//
// Each segment is color-coded (the index-key talk palette) and carries a
// label (shown as a wa-tooltip) naming what it is. Long components
// (entity / attribute / value hashes) are truncated head…tail so they
// stay distinguishable without showing the full 64 bytes. We show only
// enough to tell a key from its sibling — front coding elides leading
// components identical to the parent (see `frontCode`).

import { fromBase58, type KeyTag, tagOf } from "./key-bytes.js";

/** A rendered key segment: its label, color, and short text. */
export interface KeySegmentString {
  /** What this segment is — shown in a tooltip. */
  label: string;
  color: string;
  /** Short, distinguishable text for the component. */
  text: string;
  /** Full text, for the tooltip / title. */
  full: string;
}

// Key-component colors as Web Awesome theme tokens, so they follow the
// active theme. Tag pink, entity teal, attribute yellow/orange, value
// type blue, value cyan — the index-key palette, themed.
const COLORS = {
  tag: "var(--wa-color-pink-60, #ff8da1)",
  entity: "var(--wa-color-teal-60, #4ecbc4)",
  attribute: "var(--wa-color-yellow-60, #ffc78e)",
  type: "var(--wa-color-blue-60, #a1c8ff)",
  value: "var(--wa-color-cyan-60, #4ecbc4)",
} as const;

/** Truncate a long component to head…tail so it stays distinguishable. */
function trunc(s: string, head = 10, tail = 4): string {
  return s.length > head + tail + 1 ? `${s.slice(0, head)}…${s.slice(-tail)}` : s;
}

/** Base58 of a byte range (entity / attribute / value-ref components). */
function b58(bytes: Uint8Array): string {
  // tiny base58 encode (Bitcoin alphabet), leading-zero aware.
  const A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  const digits = [0];
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i];
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j] << 8;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  return "1".repeat(zeros) + digits.reverse().map((d) => A[d]).join("");
}

const TYPE_NAMES = [
  "Bytes", "Entity", "Boolean", "Text", "UnsignedInt", "SignedInt", "Float", "Record", "Symbol",
];

function tagLabel(tag: KeyTag): string {
  return tag === "entity"
    ? "tag — indexed by entity"
    : tag === "attribute"
      ? "tag — indexed by attribute"
      : tag === "value"
        ? "tag — indexed by value"
        : "tag — unknown index";
}

/**
 * Decode a `#<base58>` key into its ordered, labeled, color-coded
 * string segments. Ordering follows the tag, mirroring how the index
 * lays the bytes out (entity / attribute / value index differ).
 */
export function keySegments(keyStr: string): KeySegmentString[] {
  let key: Uint8Array;
  try {
    key = fromBase58(keyStr);
  } catch {
    return [{ label: "key", color: "#888", text: keyStr, full: keyStr }];
  }
  if (key.length < 162) {
    return [{ label: "key", color: "#888", text: b58(key), full: keyStr }];
  }

  const tag = tagOf(key[0]);
  const entity = b58(key.subarray(1, 65));
  const attribute = b58(key.subarray(65, 129));
  const typeByte = key[129];
  const typeName = TYPE_NAMES[typeByte] ?? String(typeByte);
  const valueRef = b58(key.subarray(130, 162));

  const tagSeg: KeySegmentString = {
    label: tagLabel(tag),
    color: COLORS.tag,
    text: String(key[0]),
    full: tagLabel(tag),
  };
  const entitySeg: KeySegmentString = {
    label: "entity",
    color: COLORS.entity,
    text: trunc(entity),
    full: entity,
  };
  const attributeSeg: KeySegmentString = {
    label: "attribute",
    color: COLORS.attribute,
    text: trunc(attribute),
    full: attribute,
  };
  const typeSeg: KeySegmentString = {
    // Show the raw type byte; the readable name is in the tooltip.
    label: `value type — ${typeName}`,
    color: COLORS.type,
    text: String(typeByte),
    full: `${typeByte} (${typeName})`,
  };
  const valueSeg: KeySegmentString = {
    label: "value",
    color: COLORS.value,
    text: trunc(valueRef),
    full: valueRef,
  };

  // Component order by index tag (value index leads with type+value).
  switch (tag) {
    case "value":
      return [tagSeg, typeSeg, valueSeg, entitySeg, attributeSeg];
    case "attribute":
      return [tagSeg, attributeSeg, entitySeg, typeSeg, valueSeg];
    default:
      return [tagSeg, entitySeg, attributeSeg, typeSeg, valueSeg];
  }
}

/**
 * Front-code a key's segments against its parent's: segments identical
 * to the parent (leading run) are marked `shared` so the row can dim or
 * elide them, showing only enough to see where this key diverges.
 */
export function frontCode(
  segs: KeySegmentString[],
  parent: KeySegmentString[] | null,
): Array<KeySegmentString & { shared: boolean }> {
  let i = 0;
  if (parent) {
    while (i < segs.length && i < parent.length && segs[i].full === parent[i].full) i++;
  }
  return segs.map((s, idx) => ({ ...s, shared: idx < i }));
}
