import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import {
  TonkCore,
  TonkError,
  FileSystemError,
  BundleError,
  ConnectionError,
} from '../dist/index.js';
import { pngBytes } from './data.js';

describe('TonkCore', () => {
  let tonk: TonkCore;

  beforeEach(async () => {
    // Create a fresh TonkCore instance for each test
    tonk = await TonkCore.create();
  });

  afterEach(() => {
    // Clean up WASM memory
    if (tonk) {
      tonk.free();
    }
  });

  test('should create TonkCore instance', async () => {
    assert.ok(tonk instanceof TonkCore);
    const peerId = await tonk.getPeerId();
    assert.ok(typeof peerId === 'string');
    assert.ok(peerId.length > 0);
  });

  test('should create and read a simple text file', async () => {
    const sampleString = 'Hello, World!';
    await tonk.createFile('/hello.txt', { sampleString });

    const retrieved = await tonk.readFile('/hello.txt');
    assert.strictEqual((retrieved.content as any).sampleString, sampleString);
    assert.strictEqual(retrieved.name, 'hello.txt');
    assert.strictEqual(retrieved.type, 'doc');
  });

  test('should create and read a file with array content', async () => {
    const content = [1, 2, 3, 'hello', { key: 'value' }];
    await tonk.createFile('/data.json', content);

    const retrieved = await tonk.readFile('/data.json');
    assert.deepStrictEqual(retrieved.content, content);
    assert.strictEqual(retrieved.name, 'data.json');
    assert.strictEqual(retrieved.type, 'doc');
  });

  // Add your tests here...
  test('should read and write bytes', async () => {
    // Your test code here...
    await tonk.createFileWithBytes(
      '/test.png',
      { mime: 'image/png' },
      pngBytes
    );
    const retrieved = await tonk.readFile('/test.png');

    // Convert base64 back to Uint8Array
    const retrievedBytes = retrieved.bytes
      ? new Uint8Array(Buffer.from(retrieved.bytes, 'base64'))
      : null;

    assert.ok(retrievedBytes, 'Should have bytes field');
    assert.equal((retrieved.content as any).mime, 'image/png');
    assert.deepStrictEqual(
      retrievedBytes,
      pngBytes,
      'Retrieved bytes should match original'
    );
  });
});

describe('Bundle', () => {
  // Add your Bundle tests here...
});

describe('Error Classes', () => {
  test('should create custom error instances', () => {
    const tonkError = new TonkError('test message', 'TEST_CODE');
    assert.ok(tonkError instanceof TonkError);
    assert.ok(tonkError instanceof Error);
    assert.strictEqual(tonkError.message, 'test message');
    assert.strictEqual(tonkError.code, 'TEST_CODE');
    assert.strictEqual(tonkError.name, 'TonkError');

    const fsError = new FileSystemError('fs error');
    assert.ok(fsError instanceof FileSystemError);
    assert.ok(fsError instanceof TonkError);
    assert.strictEqual(fsError.code, 'FILESYSTEM_ERROR');

    const bundleError = new BundleError('bundle error');
    assert.ok(bundleError instanceof BundleError);
    assert.ok(bundleError instanceof TonkError);
    assert.strictEqual(bundleError.code, 'BUNDLE_ERROR');

    const connError = new ConnectionError('connection error');
    assert.ok(connError instanceof ConnectionError);
    assert.ok(connError instanceof TonkError);
    assert.strictEqual(connError.code, 'CONNECTION_ERROR');
  });
});
