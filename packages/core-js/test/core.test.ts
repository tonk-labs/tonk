import { test, describe, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import {
  TonkCore,
  TonkError,
  FileSystemError,
  DocumentData,
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
    assert.strictEqual(retrieved.type, 'document');
  });

  test('should create and read a file with array content', async () => {
    const content = [1, 2, 3, 'hello', { key: 'value' }];
    await tonk.createFile('/data.json', content);

    const retrieved = await tonk.readFile('/data.json');
    assert.deepStrictEqual(retrieved.content, content);
    assert.strictEqual(retrieved.name, 'data.json');
    assert.strictEqual(retrieved.type, 'document');
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

    assert.ok(retrievedBytes, 'Should have bytes field');
    assert.equal((retrieved.content as any).mime, 'image/png');
    assert.deepStrictEqual(
      retrievedBytes,
      pngBytes,
      'Retrieved bytes should match original'
    );
  });

  test('watchFile callback should match readFile format for regular files', async () => {
    // Create a regular file
    const content = { message: 'Hello, World!', count: 42 };
    await tonk.createFile('/regular.txt', content);

    // Get baseline from readFile
    const readResult = await tonk.readFile('/regular.txt');

    let callbackResult: any = null;
    const watcher = await tonk.watchFile(
      '/regular.txt',
      (result: DocumentData) => {
        callbackResult = result;
      }
    );

    // Update the file to trigger the callback
    const updatedContent = { message: 'Updated!', count: 100 };
    await tonk.updateFile('/regular.txt', updatedContent);

    // Wait for callback
    await new Promise(resolve => setTimeout(resolve, 100));

    // Verify callback result matches readFile structure
    assert.ok(callbackResult, 'Callback should have been called');
    assert.strictEqual(
      callbackResult.name,
      readResult.name,
      'Name should match readFile'
    );
    assert.strictEqual(
      callbackResult.type,
      readResult.type,
      'Type should match readFile'
    );
    assert.ok(
      callbackResult.timestamps,
      'Should have timestamps like readFile'
    );
    assert.ok(
      callbackResult.timestamps.created,
      'Should have created timestamp'
    );
    assert.ok(
      callbackResult.timestamps.modified,
      'Should have modified timestamp'
    );
    assert.deepStrictEqual(
      callbackResult.content,
      updatedContent,
      'Content should match update'
    );

    // For regular files, bytes should be undefined (same as readFile)
    assert.strictEqual(
      callbackResult.bytes,
      undefined,
      'Regular files should not have bytes field'
    );

    if (watcher) await watcher.stop();
  });

  test('watchFile callback should match readFile format for files with bytes', async () => {
    // Create a file with bytes
    const content = { mime: 'image/png', description: 'Test image' };
    await tonk.createFileWithBytes('/image.png', content, pngBytes);

    // Get baseline from readFile
    const readResult = await tonk.readFile('/image.png');

    let callbackResult: any = null;
    const watcher = await tonk.watchFile('/image.png', doc => {
      callbackResult = doc;
    });

    // Update the file to trigger the callback
    const updatedContent = { mime: 'image/png', description: 'Updated image' };
    await tonk.updateFileWithBytes('/image.png', updatedContent, pngBytes);

    // Wait for callback
    await new Promise(resolve => setTimeout(resolve, 100));

    // Verify callback result matches readFile structure
    assert.ok(callbackResult, 'Callback should have been called');
    assert.strictEqual(
      callbackResult.name,
      readResult.name,
      'Name should match readFile'
    );
    assert.strictEqual(
      callbackResult.type,
      readResult.type,
      'Type should match readFile'
    );
    assert.ok(
      callbackResult.timestamps,
      'Should have timestamps like readFile'
    );
    assert.deepStrictEqual(
      callbackResult.content,
      updatedContent,
      'Content should match update'
    );

    // Verify bytes are present and in correct format (base64 string)
    assert.ok(callbackResult.bytes, 'Files with bytes should have bytes field');
    assert.strictEqual(
      typeof callbackResult.bytes,
      'string',
      'Bytes should be base64 string'
    );
    assert.strictEqual(
      callbackResult.bytes,
      readResult.bytes,
      'Bytes should match readFile format'
    );

    // Verify bytes can be converted back to original
    const retrievedBytes = new Uint8Array(
      Buffer.from(callbackResult.bytes, 'base64')
    );
    assert.deepStrictEqual(
      retrievedBytes,
      pngBytes,
      'Decoded bytes should match original'
    );

    if (watcher) await watcher.stop();
  });

  test('watchDirectory should return directory change data', async () => {
    // Create a directory to watch
    await tonk.createDirectory('/watched-dir');

    let callbackResult: any = null;

    const watcher = await tonk.watchDirectory('/watched-dir', result => {
      callbackResult = result;
      // console.log('Directory watcher callback received:', result);
    });

    // Verify watcher was created
    assert.ok(watcher, 'Directory watcher should be created');
    assert.ok(
      typeof watcher.documentId === 'function',
      'Watcher should have documentId method'
    );
    assert.ok(
      typeof watcher.stop === 'function',
      'Watcher should have stop method'
    );

    // Get the document ID
    const docId = watcher.documentId();
    assert.ok(typeof docId === 'string', 'Document ID should be a string');
    assert.ok(docId.length > 0, 'Document ID should not be empty');

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
    assert.ok(
      callbackResult,
      'Directory watcher callback should have been triggered'
    );

    // Verify the structure of the directory change data
    assert.ok(
      typeof callbackResult === 'object',
      'Callback result should be an object'
    );

    // Check if it has directory metadata structure
    if ('name' in callbackResult) {
      assert.strictEqual(
        callbackResult.name,
        'watched-dir',
        'Directory name should match'
      );
    }

    if ('type' in callbackResult) {
      assert.strictEqual(
        callbackResult.type,
        'directory',
        'Directory type should be dir'
      );
    }

    // Verify timestamps are present for directory metadata
    if ('timestamps' in callbackResult) {
      assert.ok(callbackResult.timestamps, 'Should have timestamps');
      assert.ok(
        typeof callbackResult.timestamps.created === 'number',
        'Should have created timestamp'
      );
      assert.ok(
        typeof callbackResult.timestamps.modified === 'number',
        'Should have modified timestamp'
      );
    }

    // Reset and test another change to a different file
    callbackResult = null;
    await tonk.updateFile('/watched-dir/file1.txt', {
      message: 'First file content UPDATED TOO',
      newField: 'added field',
    });
    await new Promise(resolve => setTimeout(resolve, 200));

    // Should still be receiving updates
    assert.ok(callbackResult, 'Should continue receiving directory updates');

    // Test deeply nested file change - should NOT fire callback
    callbackResult = null;

    await tonk.updateFile('/watched-dir/new-dir/test.txt', {
      message: 'Deeply nested file UPDATED',
      timestamp: Date.now(),
      level: 'nested',
    });

    await new Promise(resolve => setTimeout(resolve, 250));

    // Directory watcher should NOT fire for deeply nested changes
    assert.strictEqual(
      callbackResult,
      null,
      'Directory watcher should NOT fire for deeply nested file changes'
    );

    // Test direct child change - SHOULD fire callback
    callbackResult = null;
    await tonk.updateFile('/watched-dir/file3.txt', {
      message: 'Direct child file UPDATED',
      timestamp: Date.now(),
      level: 'direct',
    });

    await new Promise(resolve => setTimeout(resolve, 200));

    // Directory watcher SHOULD fire for direct child changes
    assert.ok(
      callbackResult !== null,
      'Directory watcher SHOULD fire for direct child changes'
    );

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
    assert.strictEqual(entries.length, 3, 'Should have exactly 3 entries');

    // Find each entry by name
    const file1 = entries.find(e => e.name === 'file1.txt');
    const file2 = entries.find(e => e.name === 'file2.json');
    const subdir = entries.find(e => e.name === 'subdir');

    // Verify file1.txt
    assert.ok(file1, 'Should find file1.txt');
    assert.strictEqual(
      file1.type,
      'document',
      'file1.txt should be a document'
    );
    assert.ok(file1.timestamps, 'file1.txt should have timestamps');
    assert.ok(
      file1.timestamps.created,
      'file1.txt should have created timestamp'
    );
    assert.ok(
      file1.timestamps.modified,
      'file1.txt should have modified timestamp'
    );
    assert.ok(
      typeof file1.pointer === 'string',
      'file1.txt should have a pointer'
    );

    // Verify file2.json
    assert.ok(file2, 'Should find file2.json');
    assert.strictEqual(
      file2.type,
      'document',
      'file2.json should be a document'
    );
    assert.ok(file2.timestamps, 'file2.json should have timestamps');
    assert.ok(
      typeof file2.pointer === 'string',
      'file2.json should have a pointer'
    );

    // Verify subdirectory
    assert.ok(subdir, 'Should find subdir');
    assert.strictEqual(
      subdir.type,
      'directory',
      'subdir should be a directory'
    );
    assert.ok(subdir.timestamps, 'subdir should have timestamps');
    assert.ok(
      typeof subdir.pointer === 'string',
      'subdir should have a pointer'
    );

    // List the subdirectory
    const subdirEntries = await tonk.listDirectory('/test-listing/subdir');
    assert.strictEqual(
      subdirEntries.length,
      1,
      'Subdirectory should have 1 entry'
    );

    const nestedFile = subdirEntries[0];
    assert.strictEqual(nestedFile.name, 'nested.txt', 'Should find nested.txt');
    assert.strictEqual(
      nestedFile.type,
      'document',
      'nested.txt should be a document'
    );
    assert.ok(nestedFile.timestamps, 'nested.txt should have timestamps');

    // Test listing empty directory
    await tonk.createDirectory('/empty-dir');
    const emptyEntries = await tonk.listDirectory('/empty-dir');
    assert.strictEqual(
      emptyEntries.length,
      0,
      'Empty directory should have no entries'
    );

    // Test listing non-existent directory should throw
    try {
      await tonk.listDirectory('/non-existent');
      assert.fail('Should throw error for non-existent directory');
    } catch (error) {
      assert.ok(
        error instanceof FileSystemError,
        'Should throw FileSystemError'
      );
    }
  });

  test('should get metadata for files and directories', async () => {
    // Create test files and directories
    await tonk.createFile('/test-file.txt', { content: 'test content' });
    await tonk.createDirectory('/test-dir');
    await tonk.createFile('/test-dir/nested-file.json', { data: [1, 2, 3] });

    // Test file metadata
    const fileMetadata = await tonk.getMetadata('/test-file.txt');
    assert.ok(fileMetadata, 'Should return metadata for existing file');
    assert.strictEqual(
      fileMetadata.name,
      'test-file.txt',
      'File name should match'
    );
    assert.strictEqual(
      fileMetadata.type,
      'document',
      'File type should be document'
    );
    assert.ok(fileMetadata.timestamps, 'Should have timestamps');
    assert.ok(
      typeof fileMetadata.timestamps.created === 'number',
      'Should have created timestamp'
    );
    assert.ok(
      typeof fileMetadata.timestamps.modified === 'number',
      'Should have modified timestamp'
    );
    assert.ok(typeof fileMetadata.pointer === 'string', 'Should have pointer');
    assert.ok(fileMetadata.pointer.length > 0, 'Pointer should not be empty');

    // Test directory metadata
    const dirMetadata = await tonk.getMetadata('/test-dir');
    assert.ok(dirMetadata, 'Should return metadata for existing directory');
    assert.strictEqual(
      dirMetadata.name,
      'test-dir',
      'Directory name should match'
    );
    assert.strictEqual(
      dirMetadata.type,
      'directory',
      'Directory type should be directory'
    );
    assert.ok(dirMetadata.timestamps, 'Directory should have timestamps');
    assert.ok(
      typeof dirMetadata.timestamps.created === 'number',
      'Directory should have created timestamp'
    );
    assert.ok(
      typeof dirMetadata.timestamps.modified === 'number',
      'Directory should have modified timestamp'
    );
    assert.ok(
      typeof dirMetadata.pointer === 'string',
      'Directory should have pointer'
    );

    // Test nested file metadata
    const nestedMetadata = await tonk.getMetadata('/test-dir/nested-file.json');
    assert.ok(nestedMetadata, 'Should return metadata for nested file');
    assert.strictEqual(
      nestedMetadata.name,
      'nested-file.json',
      'Nested file name should match'
    );
    assert.strictEqual(
      nestedMetadata.type,
      'document',
      'Nested file type should be document'
    );
    assert.ok(nestedMetadata.timestamps, 'Nested file should have timestamps');

    // Test non-existent path should throw error
    try {
      await tonk.getMetadata('/non-existent-file.txt');
      assert.fail('Should throw error for non-existent file');
    } catch (error) {
      assert.ok(
        error instanceof FileSystemError,
        'Should throw FileSystemError for non-existent file'
      );
    }

    // Test root directory metadata
    const rootMetadata = await tonk.getMetadata('/');
    assert.ok(rootMetadata, 'Should return metadata for root directory');
    assert.strictEqual(
      rootMetadata.type,
      'directory',
      'Root should be a directory'
    );

    // Test invalid path should throw error
    try {
      await tonk.getMetadata('invalid-path-without-slash');
      assert.fail('Should throw error for invalid path');
    } catch (error) {
      assert.ok(
        error instanceof FileSystemError,
        'Should throw FileSystemError for invalid path'
      );
    }
  });
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
