import type { DocumentData } from './types';

/**
 * Converts bytes from DocumentData to a UTF-8 string
 * @param documentData - The document data containing bytes
 * @returns The decoded string content
 * @throws Error if bytes are not present or decoding fails
 */
export function bytesToString(documentData: DocumentData): string {
  if (!documentData.bytes) {
    throw new Error('Document does not contain byte data');
  }

  // Decode base64 to bytes then to UTF-8 string
  const binaryString = atob(documentData.bytes);
  const bytes = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    bytes[i] = binaryString.charCodeAt(i);
  }

  const decoder = new TextDecoder();
  return decoder.decode(bytes);
}

/**
 * Converts a string to base64-encoded bytes format
 * @param stringData - The string to convert
 * @returns Base64-encoded byte string
 */
export function stringToBytes(stringData: string): string {
  const encoder = new TextEncoder();
  const bytes = encoder.encode(stringData);

  // Convert bytes to binary string in chunks to avoid call stack overflow
  // Using spread operator (...bytes) fails on large data (>~100KB)
  let binaryString = '';
  const chunkSize = 8192; // Process 8KB at a time
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binaryString += String.fromCharCode.apply(null, Array.from(chunk));
  }

  return btoa(binaryString);
}

/**
 * Converts a Uint8Array to base64 string
 * @param bytes - The byte array to convert
 * @returns Base64-encoded string
 */
export function uint8ArrayToBase64(bytes: Uint8Array): string {
  let binaryString = '';
  const chunkSize = 8192;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    const chunk = bytes.subarray(i, i + chunkSize);
    binaryString += String.fromCharCode.apply(null, Array.from(chunk));
  }
  return btoa(binaryString);
}

/**
 * Converts a base64 string to Uint8Array
 * @param base64 - The base64 string to convert
 * @returns Uint8Array of decoded bytes
 */
export function base64ToUint8Array(base64: string): Uint8Array {
  const binaryString = atob(base64);
  const bytes = new Uint8Array(binaryString.length);
  for (let i = 0; i < binaryString.length; i++) {
    bytes[i] = binaryString.charCodeAt(i);
  }
  return bytes;
}
