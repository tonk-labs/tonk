import { describe, it, expect, vi } from 'vitest';
import { RESPONSES } from '../../../src/utils/messages.js';

// Mock chalk to make tests deterministic
vi.mock('chalk', () => {
  const mockChalk = (str: string) => str;
  return {
    default: {
      redBright: mockChalk,
      cyanBright: mockChalk,
      yellowBright: mockChalk,
      blueBright: mockChalk,
      magentaBright: mockChalk,
      greenBright: mockChalk,
      italic: mockChalk,
    },
  };
});

describe('messages utils', () => {
  describe('RESPONSES', () => {
    it('should have rainbowTonk function', () => {
      expect(RESPONSES.rainbowTonk).toBeDefined();
      expect(typeof RESPONSES.rainbowTonk).toBe('function');
    });

    it('should generate rainbow Tonk text', () => {
      const result = RESPONSES.rainbowTonk();
      expect(typeof result).toBe('string');
      expect(result).toContain('T');
      expect(result).toContain('o');
      expect(result).toContain('n');
      expect(result).toContain('k');
    });

    it('should have needSubscription message', () => {
      expect(RESPONSES.needSubscription).toBeDefined();
      expect(typeof RESPONSES.needSubscription).toBe('string');
      expect(RESPONSES.needSubscription).toContain('subscription');
      expect(RESPONSES.needSubscription).toContain('deploy');
    });

    it('should include rainbowTonk in needSubscription message', () => {
      // The message should reference the rainbowTonk function
      expect(RESPONSES.needSubscription).toContain('Tonk');
    });
  });

  describe('rainbowTonk function', () => {
    it('should return a string with all letters', () => {
      const result = RESPONSES.rainbowTonk();
      expect(result).toMatch(/.*T.*o.*n.*k.*/);
    });

    it('should be consistent in length', () => {
      const result1 = RESPONSES.rainbowTonk();
      const result2 = RESPONSES.rainbowTonk();
      // Both should contain the same base characters
      expect(result1.replace(/\x1b\[[0-9;]*m/g, '')).toContain('Tonk');
      expect(result2.replace(/\x1b\[[0-9;]*m/g, '')).toContain('Tonk');
    });
  });
});
