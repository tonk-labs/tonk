// Polyfill crypto
if (typeof globalThis.crypto === 'undefined') {
  const { webcrypto } = require('crypto');
  globalThis.crypto = webcrypto;
}

// Additional fallback for macOS
if (!globalThis.crypto?.getRandomValues) {
  const crypto = require('crypto');
  globalThis.crypto = {
    ...globalThis.crypto,
    getRandomValues: function (buffer) {
      return crypto.randomFillSync(buffer);
    },
    randomUUID: function () {
      return crypto.randomUUID();
    },
  };
}

import { vi, beforeAll, afterEach, afterAll } from 'vitest';

// Global test setup
beforeAll(() => {
  // Set up environment variables for testing
  process.env.NODE_ENV = 'test';
  process.env.TONK_TEST_MODE = 'true';

  // Disable analytics during tests
  process.env.DISABLE_ANALYTICS = 'true';

  // Set up mock console to reduce noise in tests
  vi.spyOn(console, 'log').mockImplementation(() => {});
  vi.spyOn(console, 'info').mockImplementation(() => {});
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

// Reset mocks between tests
afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllEnvs();
});

// Clean up after all tests
afterAll(() => {
  vi.restoreAllMocks();
});
