import { describe, it, expect } from 'vitest';
import { CLI_VERSION } from '../../../src/utils/version.js';

describe('version utils', () => {
  describe('CLI_VERSION', () => {
    it('should export the CLI version from package.json', () => {
      expect(CLI_VERSION).toBeDefined();
      expect(typeof CLI_VERSION).toBe('string');
      expect(CLI_VERSION).toMatch(/^\d+\.\d+\.\d+/);
    });

    it('should match semantic versioning pattern', () => {
      expect(CLI_VERSION).toMatch(/^\d+\.\d+\.\d+(-.*)?$/);
    });
  });
});
