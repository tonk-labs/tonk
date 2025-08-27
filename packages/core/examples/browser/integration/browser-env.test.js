/**
 * Browser Environment Integration Tests
 * Tests browser-specific features and environment integration
 */

export class BrowserEnvironmentTests {
  constructor(wasmModule) {
    this.wasm = wasmModule;
    this.results = [];
  }

  log(message, type = 'info') {
    const timestamp = new Date().toISOString();
    this.results.push({ timestamp, message, type });
    console.log(`[${type.toUpperCase()}] ${message}`);
  }

  async runAll() {
    this.log('🔄 Starting Browser Environment Tests...', 'info');

    const tests = [
      'testIndexedDBIntegration',
      'testLocalStorageIntegration',
      'testWebWorkerIntegration',
      'testModuleLoadingPatterns',
      'testBrowserMemoryManagement',
      'testCrossOriginIsolation',
      'testServiceWorkerCompatibility',
      'testBrowserHistoryAPI',
    ];

    let passed = 0;
    const total = tests.length;

    for (const testName of tests) {
      try {
        await this[testName]();
        this.log(`✅ ${testName} passed`, 'success');
        passed++;
      } catch (error) {
        this.log(`❌ ${testName} failed: ${error.message}`, 'error');
        if (error.stack) {
          this.log(`Stack: ${error.stack}`, 'error');
        }
      }
    }

    this.log(
      `📊 Browser Environment Tests Summary: ${passed}/${total} passed`,
      passed === total ? 'success' : 'warning'
    );
    return { passed, total, results: this.results };
  }

  async testIndexedDBIntegration() {
    const dbName = 'tonk-browser-test-' + Date.now();
    const storeName = 'bundles';

    return new Promise((resolve, reject) => {
      const request = indexedDB.open(dbName, 1);

      request.onerror = () => reject(new Error('Failed to open IndexedDB'));

      request.onupgradeneeded = event => {
        const db = event.target.result;
        if (!db.objectStoreNames.contains(storeName)) {
          const store = db.createObjectStore(storeName, { keyPath: 'id' });
          store.createIndex('timestamp', 'timestamp', { unique: false });
        }
      };

      request.onsuccess = async event => {
        const db = event.target.result;

        try {
          // Create a bundle and serialize its data
          const bundle = await this.wasm.create_bundle();
          const testData = new TextEncoder().encode(
            'IndexedDB integration test'
          );
          await bundle.put('test-key', testData);

          // Simulate storing bundle metadata in IndexedDB
          const transaction = db.transaction([storeName], 'readwrite');
          const store = transaction.objectStore(storeName);

          const bundleMetadata = {
            id: 'test-bundle-' + Date.now(),
            timestamp: Date.now(),
            keys: ['test-key'],
            size: testData.length,
          };

          store.add(bundleMetadata);

          transaction.oncomplete = () => {
            // Read back the data
            const readTransaction = db.transaction([storeName], 'readonly');
            const readStore = readTransaction.objectStore(storeName);
            const getRequest = readStore.get(bundleMetadata.id);

            getRequest.onsuccess = () => {
              const result = getRequest.result;
              db.close();
              indexedDB.deleteDatabase(dbName);

              if (result && result.id === bundleMetadata.id) {
                this.log(
                  `IndexedDB integration successful, stored bundle metadata`,
                  'info'
                );
                resolve();
              } else {
                reject(
                  new Error('Failed to retrieve bundle metadata from IndexedDB')
                );
              }
            };

            getRequest.onerror = () => {
              db.close();
              indexedDB.deleteDatabase(dbName);
              reject(new Error('Failed to read from IndexedDB'));
            };
          };

          transaction.onerror = () => {
            db.close();
            indexedDB.deleteDatabase(dbName);
            reject(new Error('IndexedDB transaction failed'));
          };
        } catch (error) {
          db.close();
          indexedDB.deleteDatabase(dbName);
          reject(error);
        }
      };
    });
  }

  async testLocalStorageIntegration() {
    const prefix = 'tonk-test-' + Date.now();

    try {
      // Test storing bundle configurations
      const engine = await this.wasm.create_sync_engine();
      const peerId = await engine.getPeerId();

      // Simulate storing engine configuration
      const config = {
        peerId: peerId,
        timestamp: Date.now(),
        features: ['wasm-browser', 'atomics'],
        version: '0.1.0',
      };

      localStorage.setItem(`${prefix}-config`, JSON.stringify(config));
      localStorage.setItem(`${prefix}-peer-id`, peerId);
      localStorage.setItem(`${prefix}-last-sync`, Date.now().toString());

      // Verify storage
      const storedConfig = JSON.parse(localStorage.getItem(`${prefix}-config`));
      const storedPeerId = localStorage.getItem(`${prefix}-peer-id`);

      if (storedConfig.peerId !== peerId || storedPeerId !== peerId) {
        throw new Error('LocalStorage integration failed - data mismatch');
      }

      // Test storage limits (typical limit is 5-10MB)
      const largeDatum = 'x'.repeat(100000); // 100KB
      for (let i = 0; i < 10; i++) {
        localStorage.setItem(`${prefix}-large-${i}`, largeDatum);
      }

      // Verify large data storage
      for (let i = 0; i < 10; i++) {
        const stored = localStorage.getItem(`${prefix}-large-${i}`);
        if (stored !== largeDatum) {
          throw new Error('Large data storage failed');
        }
      }

      this.log(
        `LocalStorage integration successful, stored ${10 * 100}KB of data`,
        'info'
      );
    } finally {
      // Cleanup
      const keys = Object.keys(localStorage).filter(key =>
        key.startsWith(prefix)
      );
      keys.forEach(key => localStorage.removeItem(key));
    }
  }

  async testWebWorkerIntegration() {
    const workerCode = `
            // Import WASM module in worker context
            importScripts('../../pkg-browser/tonk_core.js');
            
            let wasmInitialized = false;
            
            self.onmessage = async function(e) {
                try {
                    if (e.data.type === 'init') {
                        // Initialize WASM in worker
                        await wasm_bindgen('../../pkg-browser/tonk_core_bg.wasm');
                        wasmInitialized = true;
                        self.postMessage({ type: 'init-complete' });
                        
                    } else if (e.data.type === 'create-engine') {
                        if (!wasmInitialized) {
                            throw new Error('WASM not initialized in worker');
                        }
                        
                        const engine = await wasm_bindgen.create_sync_engine();
                        const peerId = await engine.getPeerId();
                        
                        self.postMessage({ 
                            type: 'engine-created', 
                            peerId: peerId
                        });
                        
                    } else if (e.data.type === 'bundle-operations') {
                        if (!wasmInitialized) {
                            throw new Error('WASM not initialized in worker');
                        }
                        
                        const bundle = await wasm_bindgen.create_bundle();
                        const testData = new TextEncoder().encode('Worker test data');
                        
                        await bundle.put('worker-key', testData);
                        const retrieved = await bundle.get('worker-key');
                        
                        const success = retrieved && retrieved.length === testData.length;
                        self.postMessage({ 
                            type: 'bundle-test-complete',
                            success: success
                        });
                    }
                } catch (error) {
                    self.postMessage({ 
                        type: 'error', 
                        message: error.message,
                        stack: error.stack 
                    });
                }
            };
        `;

    return new Promise((resolve, reject) => {
      const blob = new Blob([workerCode], { type: 'application/javascript' });
      const worker = new Worker(URL.createObjectURL(blob));

      let initComplete = false;
      let engineCreated = false;
      let bundleTestComplete = false;

      worker.onmessage = function (e) {
        const { type, peerId, success, message } = e.data;

        switch (type) {
          case 'init-complete':
            initComplete = true;
            worker.postMessage({ type: 'create-engine' });
            break;

          case 'engine-created':
            if (peerId && typeof peerId === 'string') {
              engineCreated = true;
              worker.postMessage({ type: 'bundle-operations' });
            } else {
              worker.terminate();
              reject(new Error('Invalid peer ID from worker'));
            }
            break;

          case 'bundle-test-complete':
            bundleTestComplete = success;
            worker.terminate();

            if (initComplete && engineCreated && bundleTestComplete) {
              resolve();
            } else {
              reject(new Error('Web Worker integration test failed'));
            }
            break;

          case 'error':
            worker.terminate();
            reject(new Error(`Worker error: ${message}`));
            break;
        }
      };

      worker.onerror = function (error) {
        worker.terminate();
        reject(new Error(`Worker error: ${error.message}`));
      };

      // Start the test
      worker.postMessage({ type: 'init' });

      // Timeout after 10 seconds
      setTimeout(() => {
        worker.terminate();
        reject(new Error('Web Worker test timeout'));
      }, 10000);
    });
  }

  async testModuleLoadingPatterns() {
    // Test different ways of loading the WASM module

    // 1. Test dynamic import (if supported)
    try {
      // Check if dynamic import is supported
      const dynamicImportSupported = typeof eval('import') === 'function';
      if (dynamicImportSupported) {
        const dynamicModule = await import('../../pkg-browser/tonk_core.js');
        if (!dynamicModule.create_sync_engine) {
          throw new Error('Dynamic import failed to load WASM functions');
        }
        this.log('Dynamic import pattern works', 'info');
      }
    } catch (error) {
      this.log(`Dynamic import test failed: ${error.message}`, 'warning');
    }

    // 2. Test script tag loading (already loaded)
    if (typeof this.wasm.create_sync_engine === 'function') {
      this.log('Script tag loading pattern works', 'info');
    } else {
      throw new Error('Script tag loading failed');
    }

    // 3. Test fetch-based loading
    try {
      const wasmResponse = await fetch('../../pkg-browser/tonk_core_bg.wasm');
      if (!wasmResponse.ok) {
        throw new Error('Failed to fetch WASM binary');
      }
      const wasmBytes = await wasmResponse.arrayBuffer();
      if (wasmBytes.byteLength === 0) {
        throw new Error('Empty WASM binary');
      }
      this.log(
        `Fetch-based loading works, WASM size: ${wasmBytes.byteLength} bytes`,
        'info'
      );
    } catch (error) {
      this.log(`Fetch-based loading test failed: ${error.message}`, 'warning');
    }
  }

  async testBrowserMemoryManagement() {
    // Test memory usage patterns in browser environment
    const engines = [];
    const bundles = [];
    const initialMemory = performance.memory
      ? performance.memory.usedJSHeapSize
      : 0;

    try {
      // Create many objects to test memory behavior
      for (let i = 0; i < 50; i++) {
        engines.push(await this.wasm.create_sync_engine());
        bundles.push(await this.wasm.create_bundle());
      }

      // Use the objects
      for (let i = 0; i < bundles.length; i++) {
        const data = new TextEncoder().encode(`test-data-${i}`);
        await bundles[i].put(`key-${i}`, data);
      }

      const afterCreationMemory = performance.memory
        ? performance.memory.usedJSHeapSize
        : 0;

      // Cleanup (allow GC)
      engines.length = 0;
      bundles.length = 0;

      // Force garbage collection if available
      if (window.gc) {
        window.gc();
      }

      // Wait a bit for potential cleanup
      await new Promise(resolve => setTimeout(resolve, 1000));

      const afterCleanupMemory = performance.memory
        ? performance.memory.usedJSHeapSize
        : 0;

      if (performance.memory) {
        const creationIncrease = afterCreationMemory - initialMemory;
        const finalIncrease = afterCleanupMemory - initialMemory;

        this.log(
          `Memory usage: +${creationIncrease} bytes after creation, +${finalIncrease} bytes after cleanup`,
          'info'
        );

        // Memory should have decreased after cleanup (though not necessarily to original levels)
        if (finalIncrease < creationIncrease * 0.8) {
          this.log('Memory cleanup appears to be working', 'info');
        } else {
          this.log('Memory cleanup may not be optimal', 'warning');
        }
      } else {
        this.log('Memory API not available in this browser', 'info');
      }
    } catch (error) {
      throw new Error(`Memory management test failed: ${error.message}`);
    }
  }

  async testCrossOriginIsolation() {
    // Test cross-origin isolation requirements for SharedArrayBuffer
    if (window.crossOriginIsolated) {
      this.log('Cross-origin isolation is enabled', 'success');

      // Test SharedArrayBuffer availability
      if (typeof SharedArrayBuffer !== 'undefined') {
        const sab = new SharedArrayBuffer(64);
        const view = new Int32Array(sab);
        Atomics.store(view, 0, 42);
        const value = Atomics.load(view, 0);

        if (value === 42) {
          this.log(
            'SharedArrayBuffer works with cross-origin isolation',
            'info'
          );
        } else {
          throw new Error(
            'SharedArrayBuffer not working despite cross-origin isolation'
          );
        }
      } else {
        throw new Error(
          'SharedArrayBuffer not available despite cross-origin isolation'
        );
      }
    } else {
      this.log('Cross-origin isolation is NOT enabled', 'warning');
      this.log(
        'SharedArrayBuffer and atomics features will not work',
        'warning'
      );

      // Check headers that should be set
      const coepHeader = document.querySelector(
        'meta[http-equiv="Cross-Origin-Embedder-Policy"]'
      );
      const copHeader = document.querySelector(
        'meta[http-equiv="Cross-Origin-Opener-Policy"]'
      );

      if (!coepHeader || !copHeader) {
        this.log(
          'Missing required COEP/COOP headers for cross-origin isolation',
          'warning'
        );
      }
    }
  }

  async testServiceWorkerCompatibility() {
    if ('serviceWorker' in navigator) {
      try {
        // Test if service worker can be registered (don't actually register one)
        this.log('Service Worker API is available', 'info');

        // Test if our WASM module would work in a service worker context
        // This is a simplified test - in practice you'd need to test in actual SW context
        const swCompatible = await this.checkServiceWorkerWASMCompatibility();

        if (swCompatible) {
          this.log(
            'WASM module appears to be Service Worker compatible',
            'info'
          );
        } else {
          this.log(
            'WASM module may not be fully Service Worker compatible',
            'warning'
          );
        }
      } catch (error) {
        this.log(
          `Service Worker compatibility test failed: ${error.message}`,
          'warning'
        );
      }
    } else {
      this.log('Service Worker API not available', 'info');
    }
  }

  async checkServiceWorkerWASMCompatibility() {
    // Check for features that might not be available in Service Worker context
    const requiredFeatures = [
      'WebAssembly',
      'fetch',
      'Promise',
      'ArrayBuffer',
      'Uint8Array',
    ];

    for (const feature of requiredFeatures) {
      if (
        typeof window[feature] === 'undefined' &&
        typeof globalThis[feature] === 'undefined'
      ) {
        return false;
      }
    }

    return true;
  }

  async testBrowserHistoryAPI() {
    // Test interaction with browser history (for SPAs using Tonk)
    const originalLength = history.length;
    const originalState = history.state;
    const originalUrl = location.href;

    try {
      // Create engine and simulate storing its state in history
      const engine = await this.wasm.create_sync_engine();
      const peerId = await engine.getPeerId();

      const appState = {
        tonkPeerId: peerId,
        timestamp: Date.now(),
        route: '/test-route',
      };

      // Push state
      history.pushState(appState, 'Test State', '#test');

      // Verify state
      if (history.state && history.state.tonkPeerId === peerId) {
        this.log('Browser History API integration successful', 'info');

        // Test popstate event simulation
        const popstatePromise = new Promise(resolve => {
          const handler = event => {
            window.removeEventListener('popstate', handler);
            resolve(event.state);
          };
          window.addEventListener('popstate', handler);
        });

        // Go back
        history.back();

        // Wait for popstate event
        const poppedState = await Promise.race([
          popstatePromise,
          new Promise((_, reject) =>
            setTimeout(() => reject(new Error('Timeout')), 1000)
          ),
        ]);

        this.log('Popstate event handling works', 'info');
      } else {
        throw new Error('Failed to store Tonk state in browser history');
      }
    } finally {
      // Restore original state
      try {
        if (originalState) {
          history.replaceState(originalState, '', originalUrl);
        } else {
          history.replaceState(null, '', originalUrl);
        }
      } catch (e) {
        // Some browsers might restrict history manipulation
        this.log('Could not restore original history state', 'warning');
      }
    }
  }
}
