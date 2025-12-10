import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
  Bundle,
  BundleError,
  ConnectionError,
  FileSystemError,
  TonkCore,
  TonkError,
} from '../dist/index.js';
import { pngBytes } from './data.js';

describe('TonkCore', () => {
  let tonk;
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
    expect(tonk).toBeInstanceOf(TonkCore);
    const peerId = await tonk.getPeerId();
    expect(typeof peerId).toBe('string');
    expect(peerId.length).toBeGreaterThan(0);
  });
  test('should create and read a simple text file', async () => {
    const sampleString = 'Hello, World!';
    await tonk.createFile('/hello.txt', { sampleString });
    const retrieved = await tonk.readFile('/hello.txt');
    expect(retrieved.content.sampleString).toBe(sampleString);
    expect(retrieved.name).toBe('hello.txt');
    expect(retrieved.type).toBe('document');
  });
  test('should create and read a file with array content', async () => {
    const content = [1, 2, 3, 'hello', { key: 'value' }];
    await tonk.createFile('/data.json', content);
    const retrieved = await tonk.readFile('/data.json');
    expect(retrieved.content).toEqual(content);
    expect(retrieved.name).toBe('data.json');
    expect(retrieved.type).toBe('document');
  });
  test('should read and write bytes', async () => {
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
    expect(retrievedBytes).toBeTruthy();
    expect(retrieved.content.mime).toBe('image/png');
    expect(retrievedBytes).toEqual(pngBytes);
  });
  test('watchFile callback should match readFile format for regular files', async () => {
    // Create a regular file
    const content = { message: 'Hello, World!', count: 42 };
    await tonk.createFile('/regular.txt', content);
    // Get baseline from readFile
    const readResult = await tonk.readFile('/regular.txt');
    let callbackResult = null;
    const watcher = await tonk.watchFile('/regular.txt', result => {
      callbackResult = result;
    });
    // Update the file to trigger the callback
    const updatedContent = { message: 'Updated!', count: 100 };
    await tonk.updateFile('/regular.txt', updatedContent);
    // Wait for callback
    await new Promise(resolve => setTimeout(resolve, 100));
    // Verify callback result matches readFile structure
    expect(callbackResult).toBeTruthy();
    expect(callbackResult.name).toBe(readResult.name);
    expect(callbackResult.type).toBe(readResult.type);
    expect(callbackResult.timestamps).toBeTruthy();
    expect(callbackResult.timestamps.created).toBeTruthy();
    expect(callbackResult.timestamps.modified).toBeTruthy();
    expect(callbackResult.content).toEqual(updatedContent);
    // For regular files, bytes should be undefined (same as readFile)
    expect(callbackResult.bytes).toBe(undefined);
    if (watcher) await watcher.stop();
  });
  test('watchFile callback should match readFile format for files with bytes', async () => {
    // Create a file with bytes
    const content = { mime: 'image/png', description: 'Test image' };
    await tonk.createFileWithBytes('/image.png', content, pngBytes);
    // Get baseline from readFile
    const readResult = await tonk.readFile('/image.png');
    let callbackResult = null;
    const watcher = await tonk.watchFile('/image.png', doc => {
      callbackResult = doc;
    });
    // Update the file to trigger the callback
    const updatedContent = { mime: 'image/png', description: 'Updated image' };
    await tonk.updateFileWithBytes('/image.png', updatedContent, pngBytes);
    // Wait for callback
    await new Promise(resolve => setTimeout(resolve, 100));
    // Verify callback result matches readFile structure
    expect(callbackResult).toBeTruthy();
    expect(callbackResult.name).toBe(readResult.name);
    expect(callbackResult.type).toBe(readResult.type);
    expect(callbackResult.timestamps).toBeTruthy();
    expect(callbackResult.content).toEqual(updatedContent);
    // Verify bytes are present and in correct format (base64 string)
    expect(callbackResult.bytes).toBeTruthy();
    expect(typeof callbackResult.bytes).toBe('string');
    expect(callbackResult.bytes).toBe(readResult.bytes);
    // Verify bytes can be converted back to original
    const retrievedBytes = new Uint8Array(
      Buffer.from(callbackResult.bytes, 'base64')
    );
    expect(retrievedBytes).toEqual(pngBytes);
    if (watcher) await watcher.stop();
  });
  test('watchDirectory should return directory change data', async () => {
    // Create a directory to watch
    await tonk.createDirectory('/watched-dir');
    let callbackResult = null;
    const watcher = await tonk.watchDirectory('/watched-dir', result => {
      callbackResult = result;
      // console.log('Directory watcher callback received:', result);
    });
    // Verify watcher was created
    expect(watcher).toBeTruthy();
    expect(typeof watcher.documentId).toBe('function');
    expect(typeof watcher.stop).toBe('function');
    // Get the document ID
    const docId = watcher.documentId();
    expect(typeof docId).toBe('string');
    expect(docId.length).toBeGreaterThan(0);
    // First, add multiple files to the directory
    await tonk.createFile('/watched-dir/file1.txt', {
      message: 'First file content',
    });
    await tonk.createFile('/watched-dir/file2.txt', {
      message: 'Second file content',
    });
    await tonk.createFile('/watched-dir/file3.txt', {
      message: 'Third file content',
    });
    // Create nested directory and file for deeper testing
    await tonk.createDirectory('/watched-dir/new-dir');
    await tonk.createFile('/watched-dir/new-dir/test.txt', {
      message: 'Nested file content',
    });
    // Wait for initial callbacks to settle
    await new Promise(resolve => setTimeout(resolve, 300));
    // Reset callback result to capture the next update
    callbackResult = null;
    // Now modify only one of the files to see what the directory update shows
    await tonk.updateFile('/watched-dir/file2.txt', {
      message: 'Second file content UPDATED',
      timestamp: Date.now(),
    });
    // Wait for callback
    await new Promise(resolve => setTimeout(resolve, 200));
    // Verify that the callback was triggered and received data
    expect(callbackResult).toBeTruthy();
    // Verify the structure of the directory change data
    expect(typeof callbackResult).toBe('object');
    // Check if it has directory metadata structure
    if ('name' in callbackResult) {
      expect(callbackResult.name).toBe('watched-dir');
    }
    if ('type' in callbackResult) {
      expect(callbackResult.type).toBe('directory');
    }
    // Verify timestamps are present for directory metadata
    if ('timestamps' in callbackResult) {
      expect(callbackResult.timestamps).toBeTruthy();
      expect(typeof callbackResult.timestamps.created).toBe('number');
      expect(typeof callbackResult.timestamps.modified).toBe('number');
    }
    // Reset and test another change to a different file
    callbackResult = null;
    await tonk.updateFile('/watched-dir/file1.txt', {
      message: 'First file content UPDATED TOO',
      newField: 'added field',
    });
    await new Promise(resolve => setTimeout(resolve, 200));
    // Should still be receiving updates
    expect(callbackResult).toBeTruthy();
    // Test deeply nested file change - should NOT fire callback
    callbackResult = null;
    await tonk.updateFile('/watched-dir/new-dir/test.txt', {
      message: 'Deeply nested file UPDATED',
      timestamp: Date.now(),
      level: 'nested',
    });
    await new Promise(resolve => setTimeout(resolve, 250));
    // Directory watcher should NOT fire for deeply nested changes
    expect(callbackResult).toBe(null);
    // Test direct child change - SHOULD fire callback
    callbackResult = null;
    await tonk.updateFile('/watched-dir/file3.txt', {
      message: 'Direct child file UPDATED',
      timestamp: Date.now(),
      level: 'direct',
    });
    await new Promise(resolve => setTimeout(resolve, 200));
    // Directory watcher SHOULD fire for direct child changes
    expect(callbackResult).not.toBe(null);
    // Clean up
    if (watcher) await watcher.stop();
  });
  test('should list directory contents correctly', async () => {
    // Create a test directory structure
    await tonk.createDirectory('/test-listing');
    await tonk.createFile('/test-listing/file1.txt', { content: 'First file' });
    await tonk.createFile('/test-listing/file2.json', { data: [1, 2, 3] });
    await tonk.createDirectory('/test-listing/subdir');
    await tonk.createFile('/test-listing/subdir/nested.txt', { nested: true });
    // List the main directory
    const entries = await tonk.listDirectory('/test-listing');
    // Should have 3 entries (2 files + 1 subdirectory)
    expect(entries.length).toBe(3);
    // Find each entry by name
    const file1 = entries.find(e => e.name === 'file1.txt');
    const file2 = entries.find(e => e.name === 'file2.json');
    const subdir = entries.find(e => e.name === 'subdir');
    // Verify file1.txt
    expect(file1).toBeTruthy();
    expect(file1.type).toBe('document');
    expect(file1.timestamps).toBeTruthy();
    expect(file1.timestamps.created).toBeTruthy();
    expect(file1.timestamps.modified).toBeTruthy();
    expect(typeof file1.pointer).toBe('string');
    // Verify file2.json
    expect(file2).toBeTruthy();
    expect(file2.type).toBe('document');
    expect(file2.timestamps).toBeTruthy();
    expect(typeof file2.pointer).toBe('string');
    // Verify subdirectory
    expect(subdir).toBeTruthy();
    expect(subdir.type).toBe('directory');
    expect(subdir.timestamps).toBeTruthy();
    expect(typeof subdir.pointer).toBe('string');
    // List the subdirectory
    const subdirEntries = await tonk.listDirectory('/test-listing/subdir');
    expect(subdirEntries.length).toBe(1);
    const nestedFile = subdirEntries[0];
    expect(nestedFile.name).toBe('nested.txt');
    expect(nestedFile.type).toBe('document');
    expect(nestedFile.timestamps).toBeTruthy();
    // Test listing empty directory
    await tonk.createDirectory('/empty-dir');
    const emptyEntries = await tonk.listDirectory('/empty-dir');
    expect(emptyEntries.length).toBe(0);
    // Test listing non-existent directory should throw
    try {
      await tonk.listDirectory('/non-existent');
      throw new Error('Should throw error for non-existent directory');
    } catch (error) {
      expect(error).toBeInstanceOf(FileSystemError);
    }
  });
  test('should get metadata for files and directories', async () => {
    // Create test files and directories
    await tonk.createFile('/test-file.txt', { content: 'test content' });
    await tonk.createDirectory('/test-dir');
    await tonk.createFile('/test-dir/nested-file.json', { data: [1, 2, 3] });
    // Test file metadata
    const fileMetadata = await tonk.getMetadata('/test-file.txt');
    expect(fileMetadata).toBeTruthy();
    expect(fileMetadata.name).toBe('test-file.txt');
    expect(fileMetadata.type).toBe('document');
    expect(fileMetadata.timestamps).toBeTruthy();
    expect(typeof fileMetadata.timestamps.created).toBe('number');
    expect(typeof fileMetadata.timestamps.modified).toBe('number');
    expect(typeof fileMetadata.pointer).toBe('string');
    expect(fileMetadata.pointer.length).toBeGreaterThan(0);
    // Test directory metadata
    const dirMetadata = await tonk.getMetadata('/test-dir');
    expect(dirMetadata).toBeTruthy();
    expect(dirMetadata.name).toBe('test-dir');
    expect(dirMetadata.type).toBe('directory');
    expect(dirMetadata.timestamps).toBeTruthy();
    expect(typeof dirMetadata.timestamps.created).toBe('number');
    expect(typeof dirMetadata.timestamps.modified).toBe('number');
    expect(typeof dirMetadata.pointer).toBe('string');
    // Test nested file metadata
    const nestedMetadata = await tonk.getMetadata('/test-dir/nested-file.json');
    expect(nestedMetadata).toBeTruthy();
    expect(nestedMetadata.name).toBe('nested-file.json');
    expect(nestedMetadata.type).toBe('document');
    expect(nestedMetadata.timestamps).toBeTruthy();
    // Test non-existent path should throw error
    try {
      await tonk.getMetadata('/non-existent-file.txt');
      throw new Error('Should throw error for non-existent file');
    } catch (error) {
      expect(error).toBeInstanceOf(FileSystemError);
    }
    // Test root directory metadata
    const rootMetadata = await tonk.getMetadata('/');
    expect(rootMetadata).toBeTruthy();
    expect(rootMetadata.type).toBe('directory');
    // Test invalid path should throw error
    try {
      await tonk.getMetadata('invalid-path-without-slash');
      throw new Error('Should throw error for invalid path');
    } catch (error) {
      expect(error).toBeInstanceOf(FileSystemError);
    }
  });
  test('Bundle.forkToBytes should create a new fork with same contents', async () => {
    // Prepare some data in the filesystem
    await tonk.createDirectory('/fork-test');
    await tonk.createFile('/fork-test/info.json', { a: 1, b: 'two' });
    await tonk.createFileWithBytes(
      '/fork-test/image.png',
      { mime: 'image/png' },
      pngBytes
    );
    // Export current state to bundle bytes
    const originalBytes = await tonk.toBytes();
    expect(originalBytes).toBeInstanceOf(Uint8Array);
    expect(originalBytes.byteLength).toBeGreaterThan(0);
    // Create a Bundle from the bytes and record its root ID
    const originalBundle = await Bundle.fromBytes(originalBytes);
    const originalRootId = await originalBundle.getRootId();
    expect(typeof originalRootId).toBe('string');
    // Fork the current Tonk state to new bytes
    const forkBytes = await tonk.forkToBytes();
    expect(forkBytes).toBeInstanceOf(Uint8Array);
    expect(forkBytes.byteLength).toBeGreaterThan(0);
    // The fork should load as a valid bundle with a different root ID
    const forkBundle = await Bundle.fromBytes(forkBytes);
    const forkRootId = await forkBundle.getRootId();
    expect(typeof forkRootId).toBe('string');
    expect(String(forkRootId)).not.toBe(String(originalRootId));
    // Loading the fork into TonkCore should preserve file contents
    const forkTonk = await TonkCore.fromBytes(forkBytes);
    const doc = await forkTonk.readFile('/fork-test/info.json');
    expect(doc.content).toEqual({ a: 1, b: 'two' });
    const img = await forkTonk.readFile('/fork-test/image.png');
    expect(img.content.mime).toBe('image/png');
    expect(img.bytes).toBeTruthy();
    const bytesStr =
      typeof img.bytes === 'string' ? img.bytes : String(img.bytes);
    const decoded = new Uint8Array(Buffer.from(bytesStr, 'base64'));
    expect(decoded).toEqual(pngBytes);
    forkBundle.free?.();
    originalBundle.free?.();
    forkTonk.free();
  });
});
describe('Error Classes', () => {
  test('should create custom error instances', () => {
    const tonkError = new TonkError('test message', 'TEST_CODE');
    expect(tonkError).toBeInstanceOf(TonkError);
    expect(tonkError).toBeInstanceOf(Error);
    expect(tonkError.message).toBe('test message');
    expect(tonkError.code).toBe('TEST_CODE');
    expect(tonkError.name).toBe('TonkError');
    const fsError = new FileSystemError('fs error');
    expect(fsError).toBeInstanceOf(FileSystemError);
    expect(fsError).toBeInstanceOf(TonkError);
    expect(fsError.code).toBe('FILESYSTEM_ERROR');
    const bundleError = new BundleError('bundle error');
    expect(bundleError).toBeInstanceOf(BundleError);
    expect(bundleError).toBeInstanceOf(TonkError);
    expect(bundleError.code).toBe('BUNDLE_ERROR');
    const connError = new ConnectionError('connection error');
    expect(connError).toBeInstanceOf(ConnectionError);
    expect(connError).toBeInstanceOf(TonkError);
    expect(connError.code).toBe('CONNECTION_ERROR');
  });
});
