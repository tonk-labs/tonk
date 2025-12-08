import { test, describe, beforeEach, afterEach, expect } from 'bun:test';
import {
  TonkCore,
  TonkError,
  FileSystemError,
  BundleError,
  ConnectionError,
  Bundle,
} from '../dist/index.js';
import type { DocumentData } from '../dist/index.js';
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
    expect(tonk instanceof TonkCore).toBeTruthy();
    const peerId = await tonk.getPeerId();
    expect(typeof peerId === 'string').toBeTruthy();
    expect(peerId.length > 0).toBeTruthy();
  });

  test('should create and read a simple text file', async () => {
    const sampleString = 'Hello, World!';
    await tonk.createFile('/hello.txt', { sampleString });

    const retrieved = await tonk.readFile('/hello.txt');
    expect((retrieved.content as any).sampleString).toBe(sampleString);
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
    expect((retrieved.content as any).mime).toBe('image/png');
    expect(retrievedBytes).toEqual(pngBytes);
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

    let callbackResult: any = null;

    const watcher = await tonk.watchDirectory('/watched-dir', result => {
      callbackResult = result;
      // console.log('Directory watcher callback received:', result);
    });

    // Verify watcher was created
    expect(watcher).toBeTruthy();
    expect(typeof watcher.documentId === 'function').toBeTruthy();
    expect(typeof watcher.stop === 'function').toBeTruthy();

    // Get the document ID
    const docId = watcher.documentId();
    expect(typeof docId === 'string').toBeTruthy();
    expect(docId.length > 0).toBeTruthy();

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
    expect(typeof callbackResult === 'object').toBeTruthy();

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
      expect(typeof callbackResult.timestamps.created === 'number').toBeTruthy();
      expect(typeof callbackResult.timestamps.modified === 'number').toBeTruthy();
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
    expect(callbackResult !== null).toBeTruthy();

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
    expect(typeof file1.pointer === 'string').toBeTruthy();

    // Verify file2.json
    expect(file2).toBeTruthy();
    expect(file2.type).toBe('document');
    expect(file2.timestamps).toBeTruthy();
    expect(typeof file2.pointer === 'string').toBeTruthy();

    // Verify subdirectory
    expect(subdir).toBeTruthy();
    expect(subdir.type).toBe('directory');
    expect(subdir.timestamps).toBeTruthy();
    expect(typeof subdir.pointer === 'string').toBeTruthy();

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
      expect(error instanceof FileSystemError).toBeTruthy();
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
    expect(typeof fileMetadata.timestamps.created === 'number').toBeTruthy();
    expect(typeof fileMetadata.timestamps.modified === 'number').toBeTruthy();
    expect(typeof fileMetadata.pointer === 'string').toBeTruthy();
    expect(fileMetadata.pointer.length > 0).toBeTruthy();

    // Test directory metadata
    const dirMetadata = await tonk.getMetadata('/test-dir');
    expect(dirMetadata).toBeTruthy();
    expect(dirMetadata.name).toBe('test-dir');
    expect(dirMetadata.type).toBe('directory');
    expect(dirMetadata.timestamps).toBeTruthy();
    expect(typeof dirMetadata.timestamps.created === 'number').toBeTruthy();
    expect(typeof dirMetadata.timestamps.modified === 'number').toBeTruthy();
    expect(typeof dirMetadata.pointer === 'string').toBeTruthy();

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
      expect(error instanceof FileSystemError).toBeTruthy();
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
      expect(error instanceof FileSystemError).toBeTruthy();
    }
  });

  test('should rename files successfully', async () => {
    // Create a test file
    const content = { message: 'Hello, World!', count: 42 };
    await tonk.createFile('/original-file.txt', content);

    // Verify file exists at original location
    expect(await tonk.exists('/original-file.txt')).toBeTruthy();
    expect(await tonk.exists('/renamed-file.txt')).toBeFalsy();

    // Rename the file
    const result = await tonk.rename('/original-file.txt', '/renamed-file.txt');
    expect(result).toBe(true);

    // Verify file no longer exists at original location
    expect(await tonk.exists('/original-file.txt')).toBeFalsy();
    expect(await tonk.exists('/renamed-file.txt')).toBeTruthy();

    // Verify content is preserved
    const renamedFile = await tonk.readFile('/renamed-file.txt');
    expect(renamedFile.content).toEqual(content);
    expect(renamedFile.name).toBe('renamed-file.txt');
  });

  test('should rename directories successfully', async () => {
    // Create a test directory with files
    await tonk.createDirectory('/original-dir');
    await tonk.createFile('/original-dir/file1.txt', { content: 'File 1' });
    await tonk.createFile('/original-dir/file2.json', { data: [1, 2, 3] });
    await tonk.createDirectory('/original-dir/subdir');
    await tonk.createFile('/original-dir/subdir/nested.txt', { nested: true });

    // Verify directory exists at original location
    expect(await tonk.exists('/original-dir')).toBeTruthy();
    expect(await tonk.exists('/renamed-dir')).toBeFalsy();

    // Rename the directory
    const result = await tonk.rename('/original-dir', '/renamed-dir');
    expect(result).toBe(true);

    // Verify directory no longer exists at original location
    expect(await tonk.exists('/original-dir')).toBeFalsy();
    expect(await tonk.exists('/renamed-dir')).toBeTruthy();

    // Verify all nested files are accessible at new location
    const file1 = await tonk.readFile('/renamed-dir/file1.txt');
    expect(file1.content).toEqual({ content: 'File 1' });

    const file2 = await tonk.readFile('/renamed-dir/file2.json');
    expect(file2.content).toEqual({ data: [1, 2, 3] });

    const nestedFile = await tonk.readFile('/renamed-dir/subdir/nested.txt');
    expect(nestedFile.content).toEqual({ nested: true });

    // Verify old paths no longer work
    try {
      await tonk.readFile('/original-dir/file1.txt');
      throw new Error('Should not be able to read from old path');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
    }
  });

  test('should rename files with bytes successfully', async () => {
    // Create a file with bytes
    const content = { mime: 'image/png', description: 'Test image' };
    await tonk.createFileWithBytes('/original-image.png', content, pngBytes);

    // Verify file exists at original location
    expect(await tonk.exists('/original-image.png')).toBeTruthy();

    // Rename the file
    const result = await tonk.rename(
      '/original-image.png',
      '/renamed-image.png'
    );
    expect(result).toBe(true);

    // Verify file exists at new location and bytes are preserved
    const renamedFile = await tonk.readFile('/renamed-image.png');
    expect(renamedFile.content).toEqual(content);
    expect(renamedFile.name).toBe('renamed-image.png');
    expect(renamedFile.bytes).toBeTruthy();

    // Verify bytes are correct
    const retrievedBytes = new Uint8Array(
      Buffer.from(renamedFile.bytes!, 'base64')
    );
    expect(retrievedBytes).toEqual(pngBytes);
  });

  test('should throw error when trying to rename non-existent file', async () => {
    // Try to rename a file that doesn't exist
    try {
      await tonk.rename('/non-existent-file.txt', '/new-name.txt');
      throw new Error('Should throw error for non-existent file');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(error.message.includes('Path not found')).toBeTruthy();
    }

    // Verify target file was not created
    expect(await tonk.exists('/new-name.txt')).toBeFalsy();
  });

  test('should throw error when trying to rename non-existent directory', async () => {
    // Try to rename a directory that doesn't exist
    try {
      await tonk.rename('/non-existent-dir', '/new-dir');
      throw new Error('Should throw error for non-existent directory');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(error.message.includes('Path not found')).toBeTruthy();
    }

    // Verify target directory was not created
    expect(await tonk.exists('/new-dir')).toBeFalsy();
  });

  test('should throw error when trying to rename to existing file', async () => {
    // Create two files
    await tonk.createFile('/file1.txt', { content: 'File 1' });
    await tonk.createFile('/file2.txt', { content: 'File 2' });

    // Try to rename file1 to file2 (which already exists)
    try {
      await tonk.rename('/file1.txt', '/file2.txt');
      throw new Error('Should throw error when renaming to existing file');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(error.message.includes('Failed to rename')).toBeTruthy();
    }

    // Verify both files still exist with original content
    const file1 = await tonk.readFile('/file1.txt');
    expect(file1.content).toEqual({ content: 'File 1' });

    const file2 = await tonk.readFile('/file2.txt');
    expect(file2.content).toEqual({ content: 'File 2' });
  });

  test('should throw error when trying to rename to existing directory', async () => {
    // Create two directories
    await tonk.createDirectory('/dir1');
    await tonk.createDirectory('/dir2');

    // Try to rename dir1 to dir2 (which already exists)
    try {
      await tonk.rename('/dir1', '/dir2');
      throw new Error('Should throw error when renaming to existing directory');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(error.message.includes('Failed to rename')).toBeTruthy();
    }

    // Verify both directories still exist
    expect(await tonk.exists('/dir1')).toBeTruthy();
    expect(await tonk.exists('/dir2')).toBeTruthy();
  });

  test('should throw error for invalid paths', async () => {
    // Create a test file
    await tonk.createFile('/test-file.txt', { content: 'test' });

    // Test invalid old path
    try {
      await tonk.rename('invalid-path-without-slash', '/new-name.txt');
      throw new Error('Should throw error for invalid old path');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
    }

    // Test invalid new path
    try {
      await tonk.rename('/test-file.txt', 'invalid-path-without-slash');
      throw new Error('Should throw error for invalid new path');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(
        error.message.includes('must start with') ||
        error.message.includes('InvalidPath')
      ).toBeTruthy();
    }

    // Test empty paths
    try {
      await tonk.rename('', '/new-name.txt');
      throw new Error('Should throw error for empty old path');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
    }

    try {
      await tonk.rename('/test-file.txt', '');
      throw new Error('Should throw error for empty new path');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
    }
  });

  test('should throw error when trying to rename file to itself', async () => {
    // Create a test file
    await tonk.createFile('/test-file.txt', { content: 'test' });

    // Try to rename file to itself - this should throw an error
    try {
      await tonk.rename('/test-file.txt', '/test-file.txt');
      throw new Error('Should throw error when renaming file to itself');
    } catch (error) {
      expect(error instanceof FileSystemError).toBeTruthy();
      expect(error.message.includes('Cannot move')).toBeTruthy();
    }

    // Verify file still exists and is unchanged
    expect(await tonk.exists('/test-file.txt')).toBeTruthy();
    const file = await tonk.readFile('/test-file.txt');
    expect(file.content).toEqual({ content: 'test' });
  });

  test('Bundle.forkToBytes should create a new fork with same contents', async () => {
    // Prepare some data in the filesystem
    await tonk.createDirectory('/app');
    await tonk.createFile('/app/info.json', { a: 1, b: 'two' });
    await tonk.createFileWithBytes(
      '/app/image.png',
      { mime: 'image/png' },
      pngBytes
    );

    // Export current state to bundle bytes
    const originalBytes = await tonk.toBytes();
    expect(originalBytes instanceof Uint8Array).toBeTruthy();
    expect(originalBytes.byteLength > 0).toBeTruthy();

    // Create a Bundle from the bytes and record its root ID
    const originalBundle = await Bundle.fromBytes(originalBytes);
    const originalRootId = await originalBundle.getRootId();
    expect(typeof originalRootId === 'string').toBeTruthy();

    // Fork the current Tonk state to new bytes
    const forkBytes = await tonk.forkToBytes();
    expect(forkBytes instanceof Uint8Array).toBeTruthy();
    expect(forkBytes.byteLength > 0).toBeTruthy();

    // The fork should load as a valid bundle with a different root ID
    const forkBundle = await Bundle.fromBytes(forkBytes);
    const forkRootId = await forkBundle.getRootId();
    expect(typeof forkRootId === 'string').toBeTruthy();
    expect(String(forkRootId)).not.toBe(String(originalRootId));

    // Loading the fork into TonkCore should preserve file contents
    const forkTonk = await TonkCore.fromBytes(forkBytes);
    const doc = await forkTonk.readFile('/app/info.json');
    expect(doc.content).toEqual({ a: 1, b: 'two' });

    const img = await forkTonk.readFile('/app/image.png');
    expect((img.content as any).mime).toBe('image/png');
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
    expect(tonkError instanceof TonkError).toBeTruthy();
    expect(tonkError instanceof Error).toBeTruthy();
    expect(tonkError.message).toBe('test message');
    expect(tonkError.code).toBe('TEST_CODE');
    expect(tonkError.name).toBe('TonkError');

    const fsError = new FileSystemError('fs error');
    expect(fsError instanceof FileSystemError).toBeTruthy();
    expect(fsError instanceof TonkError).toBeTruthy();
    expect(fsError.code).toBe('FILESYSTEM_ERROR');

    const bundleError = new BundleError('bundle error');
    expect(bundleError instanceof BundleError).toBeTruthy();
    expect(bundleError instanceof TonkError).toBeTruthy();
    expect(bundleError.code).toBe('BUNDLE_ERROR');

    const connError = new ConnectionError('connection error');
    expect(connError instanceof ConnectionError).toBeTruthy();
    expect(connError instanceof TonkError).toBeTruthy();
    expect(connError.code).toBe('CONNECTION_ERROR');
  });
});
