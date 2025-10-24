export const PRESENCE_COLORS = [
  '#FF6B6B', // Coral Red
  '#4ECDC4', // Turquoise
  '#45B7D1', // Sky Blue
  '#FFA07A', // Light Salmon
  '#98D8C8', // Mint Green
  '#F7DC6F', // Pastel Yellow
  '#BB8FCE', // Lavender
  '#85C1E2', // Light Blue
] as const;

export type PresenceColor = typeof PRESENCE_COLORS[number];
