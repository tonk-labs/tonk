import type { DocumentData } from '@tonk/core';

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
  return btoa(String.fromCharCode(...bytes));
}
