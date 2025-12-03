export const PRESENCE_COLORS = [
  'var(--color-primary)', // Coral Red
] as const;

export type PresenceColor = (typeof PRESENCE_COLORS)[number];
