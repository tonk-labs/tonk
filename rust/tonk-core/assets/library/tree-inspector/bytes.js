// A bytes field as an array of numbers, whichever way the wire carried
// it: an array, a typed array, or dag-json's `{"/": {"bytes": base64}}`.
(self, value) => {
  if (value == null) return [];
  if (Array.isArray(value)) return value;
  if (ArrayBuffer.isView(value)) return Array.from(value);
  const encoded = value?.['/']?.bytes;
  if (typeof encoded === 'string') {
    const text = encoded.replace(/-/g, '+').replace(/_/g, '/');
    const padded = text + '='.repeat((4 - (text.length % 4)) % 4);
    try { return Array.from(atob(padded), (char) => char.charCodeAt(0)); } catch { return []; }
  }
  return [];
}
