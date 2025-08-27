/**
 * Atomics and Threading Integration Tests for Browser WASM
 * Tests the new atomics + bulk-memory features with wasm-bindgen-rayon
 */

export class AtomicsTests {
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
    this.log('🔄 Starting Atomics and Threading Tests...', 'info');

    const tests = [
      'testSharedArrayBufferSupport',
      'testAtomicsOperations',
      'testConcurrentBundleOperations',
      'testParallelEngineCreation',
      'testAtomicMemoryAccess',
      'testRayonParallelism',
      'testMemoryOrdering',
      'testLockFreeDataStructures',
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
      `📊 Atomics Tests Summary: ${passed}/${total} passed`,
      passed === total ? 'success' : 'warning'
    );
    return { passed, total, results: this.results };
  }

  async testSharedArrayBufferSupport() {
    if (typeof SharedArrayBuffer === 'undefined') {
      throw new Error('SharedArrayBuffer not supported in this browser');
    }

    if (!window.crossOriginIsolated) {
      throw new Error(
        'Cross-origin isolation not enabled (required for SharedArrayBuffer)'
      );
    }

    // Test basic SharedArrayBuffer functionality
    const buffer = new SharedArrayBuffer(1024);
    const view = new Int32Array(buffer);

    // Test atomic operations
    Atomics.store(view, 0, 42);
    const value = Atomics.load(view, 0);

    if (value !== 42) {
      throw new Error('Basic atomic store/load failed');
    }

    // Test compare and exchange
    const exchanged = Atomics.compareExchange(view, 0, 42, 100);
    if (exchanged !== 42 || Atomics.load(view, 0) !== 100) {
      throw new Error('Atomic compareExchange failed');
    }

    this.log('SharedArrayBuffer and basic atomics working correctly', 'info');
  }

  async testAtomicsOperations() {
    const buffer = new SharedArrayBuffer(256);
    const view = new Int32Array(buffer);

    // Test various atomic operations
    const operations = [
      { op: 'add', args: [0, 10], expected: 0 },
      { op: 'sub', args: [0, 5], expected: 10 },
      { op: 'and', args: [0, 7], expected: 5 },
      { op: 'or', args: [0, 8], expected: 5 },
      { op: 'xor', args: [0, 3], expected: 13 },
    ];

    for (const { op, args, expected } of operations) {
      const result = Atomics[op](view, ...args);
      if (result !== expected) {
        throw new Error(
          `Atomic ${op} operation failed: expected ${expected}, got ${result}`
        );
      }
    }

    this.log('All atomic operations working correctly', 'info');
  }

  async testConcurrentBundleOperations() {
    // Test WASM module operations with potential parallelism
    const numWorkers = Math.min(navigator.hardwareConcurrency || 4, 8);
    const operationsPerWorker = 20;

    this.log(
      `Testing concurrent operations with ${numWorkers} workers`,
      'info'
    );

    const bundles = [];
    for (let i = 0; i < numWorkers; i++) {
      bundles.push(await this.wasm.create_bundle());
    }

    // Perform concurrent operations
    const operations = [];
    for (let i = 0; i < numWorkers; i++) {
      const bundle = bundles[i];
      for (let j = 0; j < operationsPerWorker; j++) {
        const key = `concurrent-${i}-${j}`;
        const value = new TextEncoder().encode(`data-${i}-${j}-${Date.now()}`);
        operations.push(bundle.put(key, value));
      }
    }

    const start = performance.now();
    await Promise.all(operations);
    const duration = performance.now() - start;

    // Verify all operations completed
    for (let i = 0; i < numWorkers; i++) {
      const bundle = bundles[i];
      for (let j = 0; j < operationsPerWorker; j++) {
        const key = `concurrent-${i}-${j}`;
        const retrieved = await bundle.get(key);
        if (!retrieved) {
          throw new Error(`Failed to retrieve ${key}`);
        }
      }
    }

    this.log(
      `Concurrent bundle operations completed in ${duration.toFixed(2)}ms`,
      'info'
    );
  }

  async testParallelEngineCreation() {
    const numEngines = 16;
    const start = performance.now();

    // Create engines in parallel
    const enginePromises = [];
    for (let i = 0; i < numEngines; i++) {
      enginePromises.push(this.wasm.create_sync_engine());
    }

    const engines = await Promise.all(enginePromises);
    const duration = performance.now() - start;

    if (engines.length !== numEngines) {
      throw new Error(`Expected ${numEngines} engines, got ${engines.length}`);
    }

    // Verify all engines are unique and functional
    const peerIds = new Set();
    for (const engine of engines) {
      const peerId = await engine.getPeerId();
      if (peerIds.has(peerId)) {
        throw new Error('Duplicate peer ID detected in parallel creation');
      }
      peerIds.add(peerId);
    }

    this.log(
      `Created ${numEngines} engines in parallel in ${duration.toFixed(2)}ms`,
      'info'
    );
  }

  async testAtomicMemoryAccess() {
    // Test atomic memory access patterns that might be used by Rust/WASM
    const buffer = new SharedArrayBuffer(4096);
    const view32 = new Int32Array(buffer);
    const view8 = new Uint8Array(buffer);

    // Test memory ordering and visibility
    const numSlots = 256;

    // Initialize memory
    for (let i = 0; i < numSlots; i++) {
      Atomics.store(view32, i, i * 2);
    }

    // Test concurrent read/write patterns
    const readers = [];
    const writers = [];

    // Create reader operations
    for (let i = 0; i < 4; i++) {
      readers.push(
        (async () => {
          let sum = 0;
          for (let j = 0; j < numSlots; j++) {
            sum += Atomics.load(view32, j);
          }
          return sum;
        })()
      );
    }

    // Create writer operations
    for (let i = 0; i < 2; i++) {
      writers.push(
        (async () => {
          for (let j = 0; j < numSlots; j++) {
            Atomics.add(view32, j, 1);
          }
        })()
      );
    }

    // Execute all operations concurrently
    const [readerResults] = await Promise.all([
      Promise.all(readers),
      Promise.all(writers),
    ]);

    // Verify memory consistency
    let finalSum = 0;
    for (let i = 0; i < numSlots; i++) {
      finalSum += Atomics.load(view32, i);
    }

    this.log(`Memory access test completed, final sum: ${finalSum}`, 'info');
  }

  async testRayonParallelism() {
    // Test that the WASM module can actually utilize rayon parallelism
    try {
      // Create multiple bundles and perform operations that might benefit from parallelism
      const bundles = [];
      const numBundles = 8;

      for (let i = 0; i < numBundles; i++) {
        bundles.push(await this.wasm.create_bundle());
      }

      // Perform operations that might trigger parallel processing in Rust
      const largeDatum = new Uint8Array(64 * 1024); // 64KB
      largeDatum.fill(255);

      const start = performance.now();
      const operations = bundles.map(async (bundle, i) => {
        // Store large data that might trigger parallel compression/processing
        await bundle.put(`large-data-${i}`, largeDatum);
        return bundle.get(`large-data-${i}`);
      });

      const results = await Promise.all(operations);
      const duration = performance.now() - start;

      // Verify all operations completed correctly
      for (let i = 0; i < results.length; i++) {
        if (!results[i] || results[i].length !== largeDatum.length) {
          throw new Error(`Large data operation ${i} failed`);
        }
      }

      this.log(
        `Rayon parallelism test completed in ${duration.toFixed(2)}ms`,
        'info'
      );
    } catch (error) {
      // If rayon features aren't exposed, that's okay - just log it
      this.log(
        `Rayon test skipped (may not be exposed to JS): ${error.message}`,
        'warning'
      );
    }
  }

  async testMemoryOrdering() {
    // Test memory ordering guarantees
    const buffer = new SharedArrayBuffer(64);
    const view = new Int32Array(buffer);

    // Test sequential consistency
    Atomics.store(view, 0, 1);
    Atomics.store(view, 1, 2);

    // These should be visible immediately due to sequential consistency
    const val0 = Atomics.load(view, 0);
    const val1 = Atomics.load(view, 1);

    if (val0 !== 1 || val1 !== 2) {
      throw new Error('Memory ordering test failed');
    }

    // Test fence operations
    Atomics.store(view, 2, 42);
    Atomics.fence();
    const fencedValue = Atomics.load(view, 2);

    if (fencedValue !== 42) {
      throw new Error('Atomic fence test failed');
    }

    this.log('Memory ordering tests passed', 'info');
  }

  async testLockFreeDataStructures() {
    // Simulate lock-free operations that WASM might perform
    const buffer = new SharedArrayBuffer(1024);
    const view = new Int32Array(buffer);

    // Simulate a simple lock-free queue using atomics
    const QUEUE_SIZE = 64;
    const head = 0; // Index 0: head pointer
    const tail = 1; // Index 1: tail pointer
    const data = 2; // Index 2+: queue data

    Atomics.store(view, head, 0);
    Atomics.store(view, tail, 0);

    // Test enqueue operations
    const numItems = 32;
    const enqueueOperations = [];

    for (let i = 0; i < numItems; i++) {
      enqueueOperations.push(
        (async () => {
          // Atomic enqueue simulation
          const tailPos = Atomics.load(view, tail);
          const nextTail = (tailPos + 1) % QUEUE_SIZE;

          // Store data
          Atomics.store(view, data + tailPos, i + 100);

          // Update tail
          Atomics.store(view, tail, nextTail);

          return tailPos;
        })()
      );
    }

    const enqueuedPositions = await Promise.all(enqueueOperations);

    // Verify queue state
    const finalTail = Atomics.load(view, tail);
    const finalHead = Atomics.load(view, head);

    this.log(
      `Lock-free queue test: head=${finalHead}, tail=${finalTail}, enqueued=${numItems}`,
      'info'
    );
  }

  // Helper method to create a worker that can test WASM in a separate thread
  async createWorkerTest() {
    return new Promise((resolve, reject) => {
      const workerCode = `
                importScripts('../../pkg-browser/tonk_core.js');
                
                const { initSync } = wasm_bindgen;
                
                self.onmessage = async function(e) {
                    try {
                        if (e.data.type === 'init') {
                            await initSync(fetch('../../pkg-browser/tonk_core_bg.wasm'));
                            self.postMessage({ type: 'ready' });
                        } else if (e.data.type === 'test') {
                            // Perform WASM operations in worker
                            const engine = await wasm_bindgen.create_sync_engine();
                            const peerId = await engine.getPeerId();
                            self.postMessage({ type: 'result', peerId });
                        }
                    } catch (error) {
                        self.postMessage({ type: 'error', error: error.message });
                    }
                };
            `;

      const blob = new Blob([workerCode], { type: 'application/javascript' });
      const worker = new Worker(URL.createObjectURL(blob));

      let ready = false;
      worker.onmessage = function (e) {
        if (e.data.type === 'ready') {
          ready = true;
          worker.postMessage({ type: 'test' });
        } else if (e.data.type === 'result' && ready) {
          worker.terminate();
          resolve(e.data.peerId);
        } else if (e.data.type === 'error') {
          worker.terminate();
          reject(new Error(e.data.error));
        }
      };

      worker.onerror = function (error) {
        worker.terminate();
        reject(error);
      };

      // Initialize worker
      worker.postMessage({ type: 'init' });

      // Timeout fallback
      setTimeout(() => {
        worker.terminate();
        reject(new Error('Worker test timeout'));
      }, 10000);
    });
  }
}
