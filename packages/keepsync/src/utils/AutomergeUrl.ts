import SHA256 from 'crypto-js/sha256.js';
import pkg from 'crypto-js';
const {enc} = pkg;

/**
 * Converts a string to a deterministic UUID v4 synchronously.
 * Uses SHA-256 from crypto-js for cryptographically secure hashing.
 *
 * @param input - The input string to convert
 * @returns A UUID v4 string that is deterministically generated from the input
 */
export function stringToUuidV4(input: string): string {
  // Generate SHA-256 hash of input
  const hash = SHA256(input);
  // Get first 16 bytes (128 bits) of hash as hex
  const hashHex = hash.toString(enc.Hex).slice(0, 32);

  // Convert to byte array
  const bytes = new Uint8Array(16);
  for (let i = 0; i < 16; i++) {
    bytes[i] = parseInt(hashHex.slice(i * 2, i * 2 + 2), 16);
  }

  // Set version bits for v4 UUID (0100)
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  // Set variant bits (10)
  bytes[8] = (bytes[8] & 0x3f) | 0x80;

  // Convert to hex and format as UUID
  const uuid = Array.from(bytes)
    .map(b => b.toString(16).padStart(2, '0'))
    .join('');

  return `${uuid.slice(0, 8)}-${uuid.slice(8, 12)}-${uuid.slice(
    12,
    16,
  )}-${uuid.slice(16, 20)}-${uuid.slice(20)}`;
}
