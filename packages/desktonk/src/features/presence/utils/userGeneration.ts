import {
  adjectives,
  animals,
  uniqueNamesGenerator,
} from 'unique-names-generator';
import { PRESENCE_COLORS } from '../../../lib/constants/presenceColors';

/**
 * Generate deterministic color from userId using hash
 */
export const getUserColor = (userId: string): string => {
  const hash = userId
    .split('')
    .reduce((acc, char) => char.charCodeAt(0) + ((acc << 5) - acc), 0);
  return PRESENCE_COLORS[Math.abs(hash) % PRESENCE_COLORS.length];
};

/**
 * Generate deterministic animal name from userId
 */
export const getUserName = (userId: string): string => {
  return uniqueNamesGenerator({
    dictionaries: [adjectives, animals],
    seed: userId,
    separator: ' ',
    style: 'capital',
  });
};

/**
 * Get initials from animal name (first letter of each word)
 */
export const getInitials = (name: string): string => {
  return name
    .split(' ')
    .map(word => word[0])
    .join('')
    .toUpperCase();
};

/**
 * Get or create persistent user ID in localStorage
 */
export const initializeUserId = (): string => {
  const STORAGE_KEY = 'tonk-user-id';
  let userId = localStorage.getItem(STORAGE_KEY);

  console.log('[UserGeneration] initializeUserId() called');
  console.log('[UserGeneration] Current localStorage value:', userId);

  if (!userId) {
    userId = crypto.randomUUID();
    localStorage.setItem(STORAGE_KEY, userId);
    console.log('[UserGeneration] Created new userId:', userId);
  } else {
    console.log('[UserGeneration] Using existing userId:', userId);
  }

  return userId;
};
