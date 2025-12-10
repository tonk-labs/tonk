/**
 * Shared test utilities for Tonk Core integration tests
 */

const fs = require('fs');
const path = require('path');
const tmp = require('tmp');

/**
 * Initialize WASM module for testing
 */
async function initWasm() {
  try {
    const wasmPath = path.resolve(__dirname, '../../pkg-node');
    const wasmModule = require(path.join(wasmPath, 'tonk_core.js'));
    return wasmModule;
  } catch (error) {
    throw new Error(
      `Failed to load WASM module: ${error.message}. Make sure to run 'npm run build:wasm' first.`
    );
  }
}

/**
 * Create a temporary directory for test files
 */
function createTempDir() {
  return tmp.dirSync({ unsafeCleanup: true });
}

/**
 * Create a temporary file with content
 */
function createTempFile(content = '', extension = '.txt') {
  const tmpFile = tmp.fileSync({ postfix: extension });
  if (content) {
    fs.writeFileSync(tmpFile.name, content);
  }
  return tmpFile;
}

/**
 * Generate test data for various scenarios
 */
const TestData = {
  simpleText: 'Hello, World!',
  jsonConfig: JSON.stringify({ theme: 'dark', language: 'en' }, null, 2),
  binaryData: new Uint8Array([1, 2, 3, 4, 5, 255, 254, 253]),
  largeText: 'x'.repeat(1024 * 1024), // 1MB of text

  // File structure for VFS testing
  fileStructure: [
    { path: '/README.md', content: '# Test Project\n\nThis is a test.' },
    { path: '/src/main.js', content: 'console.log("Hello from main!");' },
    {
      path: '/src/utils/helper.js',
      content: 'export const helper = () => "help";',
    },
    { path: '/config/settings.json', content: '{"debug": true}' },
    { path: '/data/users.csv', content: 'name,age\nAlice,30\nBob,25' },
  ],
};

/**
 * Wait for a condition to be true with timeout
 */
async function waitFor(condition, timeoutMs = 5000, intervalMs = 100) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await condition()) {
      return true;
    }
    await sleep(intervalMs);
  }
  throw new Error(`Condition not met within ${timeoutMs}ms`);
}

/**
 * Sleep for specified milliseconds
 */
function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Generate a random peer ID
 */
function generatePeerId() {
  return `test-peer-${Math.random().toString(36).substr(2, 9)}`;
}

/**
 * Assert that two Uint8Arrays are equal
 */
function assertUint8ArraysEqual(actual, expected) {
  if (actual.length !== expected.length) {
    throw new Error(
      `Array lengths differ: ${actual.length} !== ${expected.length}`
    );
  }
  for (let i = 0; i < actual.length; i++) {
    if (actual[i] !== expected[i]) {
      throw new Error(
        `Arrays differ at index ${i}: ${actual[i]} !== ${expected[i]}`
      );
    }
  }
}

/**
 * Create a bundle with test data
 */
async function createTestBundle(wasm, files = TestData.fileStructure) {
  const bundle = await wasm.create_bundle();

  for (const file of files) {
    const content = new TextEncoder().encode(file.content);
    await bundle.put(file.path, content);
  }

  return bundle;
}

/**
 * Performance measurement utility
 */
class PerfTimer {
  constructor(name = 'operation') {
    this.name = name;
    this.start = process.hrtime.bigint();
  }

  stop() {
    const end = process.hrtime.bigint();
    const durationMs = Number(end - this.start) / 1_000_000;
    console.log(`${this.name} took ${durationMs.toFixed(2)}ms`);
    return durationMs;
  }
}

/**
 * Retry a function with exponential backoff
 */
async function retry(fn, maxAttempts = 3, baseDelayMs = 100) {
  let lastError;

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (error) {
      lastError = error;
      if (attempt === maxAttempts) break;

      const delay = baseDelayMs * 2 ** (attempt - 1);
      await sleep(delay);
    }
  }

  throw lastError;
}

module.exports = {
  initWasm,
  createTempDir,
  createTempFile,
  TestData,
  waitFor,
  sleep,
  generatePeerId,
  assertUint8ArraysEqual,
  createTestBundle,
  PerfTimer,
  retry,
};
