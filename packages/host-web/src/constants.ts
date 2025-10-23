/**
 * Allowed origins for cross-origin postMessage communication
 * Used for drag-and-drop file handling between parent and iframe contexts
 */
export const ALLOWED_ORIGINS: readonly string[] = [
  'https://tonk.xyz',
  'https://www.tonk.xyz',
  'http://localhost:3000',
  'http://localhost:4000',
  'http://localhost:5173',
  'http://localhost:5174',
];
