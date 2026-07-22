// Base64 <-> bytes, for carrying the engine's binary workbook
// serialization inside the text-only `content` envelope. `btoa`/`atob`
// are available in every runtime this bundle targets (browsers and
// Node ≥ 16, which the node test runner uses).

/** Encode bytes as base64. Chunked so `String.fromCharCode` never sees
 *  an argument list large enough to overflow the engine's limit — the
 *  workbook serialization of a real sheet runs to hundreds of KB. */
export function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK = 0x8000;
  let binary = "";
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}

/** Decode base64 into bytes, or null when `text` isn't base64.
 *  Whitespace-tolerant (an envelope body may pick up incidental
 *  wrapping in transit); a null result lets the caller degrade —
 *  a corrupt body must not throw out of the content path. */
export function base64ToBytes(text: string): Uint8Array | null {
  const compact = text.replace(/\s+/g, "");
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(compact) || compact.length % 4 === 1) {
    return null;
  }
  try {
    const binary = atob(compact);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      out[i] = binary.charCodeAt(i);
    }
    return out;
  } catch {
    return null;
  }
}
