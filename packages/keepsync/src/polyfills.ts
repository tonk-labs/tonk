import {logger} from './utils/logger.js';
import fakeIndexDB from 'fake-indexeddb';
/**
 * Automatic polyfills for Node.js environment
 * This file is automatically imported by the main module
 */

// Only apply polyfills in Node.js environment
if (typeof window === 'undefined') {
  logger.info('We are in a node environment, running browser polyfills.');
  // Polyfill for IndexedDB
  try {
    global.indexedDB = fakeIndexDB;
  } catch (err) {
    console.warn('Could not load fake-indexeddb', err);
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
        .catch(err => {
          console.warn('Could not load node-fetch', err);
          // Create dummy fetch that will show a clear error message
          globalThis.fetch = () => {
            throw new Error(
              'node-fetch is required but not available. Please install it with: npm install node-fetch',
            );
          };
        });
    } catch (err) {
      console.warn('Error initializing node-fetch', err);
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
      console.warn('Could not polyfill Blob', err);
    }
  }

  // Simple URL.createObjectURL mock if needed
  if (global.URL && !global.URL.createObjectURL) {
    global.URL.createObjectURL = () => 'blob:mock-url';
    global.URL.revokeObjectURL = () => {};
  }

  // WebSocket polyfill
  if (!global.WebSocket) {
    try {
      const WebSocket = require('ws');
      global.WebSocket = WebSocket;
    } catch (err) {
      console.warn('Could not polyfill WebSocket', err);
    }
  }

  // Mock localStorage
  if (!global.localStorage) {
    class LocalStorage {
      private store: Record<string, string> = {};

      getItem(key: string): string | null {
        return this.store[key] || null;
      }

      setItem(key: string, value: string): void {
        this.store[key] = String(value);
      }

      removeItem(key: string): void {
        delete this.store[key];
      }

      clear(): void {
        this.store = {};
      }

      key(index: number): string | null {
        return Object.keys(this.store)[index] || null;
      }

      get length(): number {
        return Object.keys(this.store).length;
      }
    }

    global.localStorage = new LocalStorage() as unknown as Storage;
  }

  // Mock sessionStorage
  if (!global.sessionStorage && global.localStorage) {
    global.sessionStorage = global.localStorage;
  }

  // Create minimal window object for compatibility
  if (!global.window) {
    global.window = global as unknown as Window & typeof globalThis;
  }
}

// Export nothing
export {};
