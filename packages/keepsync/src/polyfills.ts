/**
 * Automatic polyfills for Node.js environment
 * This file is automatically imported by the main module
 */

// Only apply polyfills in Node.js environment
if (typeof window === 'undefined') {
  // Polyfill for IndexedDB (already handled in index.ts)
  try {
    const fakeIndexedDB = require('fake-indexeddb');
    if (!global.indexedDB) {
      global.indexedDB = fakeIndexedDB;
    }
  } catch (err) {
    // Silent fail
  }

  // Polyfill for fetch API with node-fetch
  if (!global.fetch) {
    try {
      // For ESM compatibility
      import('node-fetch')
        .then(({default: fetch, Headers, Request, Response}) => {
          // Use type assertions to make node-fetch types compatible with browser fetch API
          global.fetch = fetch as unknown as typeof global.fetch;
          global.Headers = Headers as unknown as typeof global.Headers;
          global.Request = Request as unknown as typeof global.Request;
          global.Response = Response as unknown as typeof global.Response;
        })
        .catch(() => {
          // Create dummy fetch that will show a clear error message
          globalThis.fetch = () => {
            throw new Error(
              'node-fetch is required but not available. Please install it with: npm install node-fetch',
            );
          };
        });
    } catch (err) {
      // Create dummy fetch that will show a clear error message
      globalThis.fetch = () => {
        throw new Error(
          'node-fetch is required but not available. Please install it with: npm install node-fetch',
        );
      };
    }
  }

  // Simple Blob mock if needed
  if (!globalThis.Blob) {
    try {
      // Try Node.js built-in Blob
      const {Blob} = require('buffer');
      global.Blob = Blob;
    } catch (err) {
      // Silent fail
    }
  }

  // Simple URL.createObjectURL mock if needed
  if (global.URL && !global.URL.createObjectURL) {
    global.URL.createObjectURL = () => 'blob:mock-url';
    global.URL.revokeObjectURL = () => {};
  }
}

// Export nothing
export {};
