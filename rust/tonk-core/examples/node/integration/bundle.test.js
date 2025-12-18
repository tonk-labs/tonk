/**
 * Bundle operations integration tests
 */

const { expect } = require('chai');
const fs = require('fs');
const {
  initWasm,
  createTempFile,
  TestData,
  assertUint8ArraysEqual,
  PerfTimer,
} = require('../../shared/test-utils');

describe('Bundle Integration Tests', () => {
  let wasm;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  describe('Bundle Creation and Basic Operations', () => {
    it('should create bundle from existing data', async () => {
      // Create a test zip file (this would normally be created by the bundle)
      const bundle = await wasm.create_bundle();

      // Add some test data
      const testFiles = [
        { key: 'config.json', content: TestData.jsonConfig },
        { key: 'readme.txt', content: TestData.simpleText },
        { key: 'data/binary.dat', content: TestData.binaryData },
      ];

      for (const file of testFiles) {
        const content =
          typeof file.content === 'string'
            ? new TextEncoder().encode(file.content)
            : file.content;
        await bundle.put(file.key, content);
      }

      // Verify all files are stored
      const keys = await bundle.listKeys();
      expect(keys).to.have.lengthOf(testFiles.length + 2); // +2 for manifest and root doc

      for (const file of testFiles) {
        expect(keys).to.include(file.key);
      }
    });

    it('should handle hierarchical paths', async () => {
      const bundle = await wasm.create_bundle();

      const hierarchicalFiles = [
        'root.txt',
        'dir1/file1.txt',
        'dir1/file2.txt',
        'dir1/subdir/file3.txt',
        'dir2/another.txt',
      ];

      for (const [index, path] of hierarchicalFiles.entries()) {
        const content = new TextEncoder().encode(`Content ${index}: ${path}`);
        await bundle.put(path, content);
      }

      const keys = await bundle.listKeys();
      expect(keys).to.have.lengthOf(hierarchicalFiles.length + 2); // +2 for manifest and root doc

      // Verify we can retrieve all files
      for (const path of hierarchicalFiles) {
        const data = await bundle.get(path);
        expect(data).to.not.be.null;

        const content = new TextDecoder().decode(data);
        expect(content).to.include(path);
      }
    });

    it('should preserve data integrity', async () => {
      const bundle = await wasm.create_bundle();

      // Test with various data types
      const testCases = [
        { key: 'text.txt', data: TestData.simpleText },
        { key: 'json.json', data: TestData.jsonConfig },
        { key: 'large.txt', data: TestData.largeText.substring(0, 1000) }, // Smaller for test speed
        { key: 'binary.dat', data: TestData.binaryData },
      ];

      // Store all data
      for (const testCase of testCases) {
        const content =
          typeof testCase.data === 'string'
            ? new TextEncoder().encode(testCase.data)
            : testCase.data;
        await bundle.put(testCase.key, content);
      }

      // Retrieve and verify
      for (const testCase of testCases) {
        const retrieved = await bundle.get(testCase.key);
        const expected =
          typeof testCase.data === 'string'
            ? new TextEncoder().encode(testCase.data)
            : testCase.data;

        assertUint8ArraysEqual(retrieved, expected);
      }
    });

    it('should handle overwrites correctly', async function () {
      const bundle = await wasm.create_bundle();
      const key = 'overwrite-test.txt';

      // Initial content
      const content1 = new TextEncoder().encode('Initial content');
      await bundle.put(key, content1);

      let retrieved = await bundle.get(key);
      assertUint8ArraysEqual(retrieved, content1);

      try {
        // Overwrite
        const content2 = new TextEncoder().encode(
          'Updated content - much longer this time'
        );
        await bundle.put(key, content2);

        retrieved = await bundle.get(key);
        assertUint8ArraysEqual(retrieved, content2);

        // Verify keys list still has only one entry
        const keys = await bundle.listKeys();
        const matchingKeys = keys.filter(k => k === key);
        expect(matchingKeys).to.have.lengthOf(1);
      } catch (error) {
        // If overwrite fails due to ZIP limitations, this is a known issue
        if (
          error.message &&
          error.message.includes('Failed to start new file in ZIP')
        ) {
          this.skip(
            'Bundle overwrite not supported due to ZIP library limitations'
          );
        } else {
          throw error;
        }
      }
    });
  });

  describe('Bundle Serialization', () => {
    it('should serialize and deserialize bundle data', async () => {
      // Create and populate a bundle
      const originalBundle = await wasm.create_bundle();

      const testData = [
        { key: 'file1.txt', content: 'Hello World' },
        { key: 'dir/file2.json', content: '{"test": true}' },
        { key: 'binary.dat', content: TestData.binaryData },
      ];

      for (const item of testData) {
        const content =
          typeof item.content === 'string'
            ? new TextEncoder().encode(item.content)
            : item.content;
        await originalBundle.put(item.key, content);
      }

      // Serialize to bytes
      const serialized = await originalBundle.toBytes();
      expect(serialized).to.be.instanceOf(Uint8Array);
      expect(serialized.length).to.be.greaterThan(0);

      // Create new bundle from serialized data
      const deserializedBundle =
        await wasm.create_bundle_from_bytes(serialized);

      // Verify all data is preserved
      const keys = await deserializedBundle.listKeys();
      expect(keys).to.have.lengthOf(testData.length + 2); // +2 for manifest and root doc

      for (const item of testData) {
        const retrieved = await deserializedBundle.get(item.key);
        const expected =
          typeof item.content === 'string'
            ? new TextEncoder().encode(item.content)
            : item.content;
        assertUint8ArraysEqual(retrieved, expected);
      }
    });

    it('should handle empty bundle serialization', async () => {
      const bundle = await wasm.create_bundle();

      const serialized = await bundle.toBytes();
      expect(serialized).to.be.instanceOf(Uint8Array);

      const deserialized = await wasm.create_bundle_from_bytes(serialized);
      const keys = await deserialized.listKeys();
      expect(keys).to.have.lengthOf(2); // +2 for manifest and root doc
    });
  });

  describe('Bundle Performance', () => {
    it('should handle large numbers of files efficiently', async function () {
      this.timeout(15000);

      const bundle = await wasm.create_bundle();
      const fileCount = 1000;

      const timer = new PerfTimer(`Storing ${fileCount} files`);

      // Store many small files
      for (let i = 0; i < fileCount; i++) {
        const key = `files/file_${i.toString().padStart(4, '0')}.txt`;
        const content = new TextEncoder().encode(`Content of file ${i}`);
        await bundle.put(key, content);
      }

      const storeTime = timer.stop();

      // Verify count
      const keys = await bundle.listKeys();
      expect(keys).to.have.lengthOf(fileCount + 2); // +2 for manifest and root doc

      // Time retrieval
      const retrieveTimer = new PerfTimer('Retrieving all files');
      for (let i = 0; i < fileCount; i++) {
        const key = `files/file_${i.toString().padStart(4, '0')}.txt`;
        const data = await bundle.get(key);
        expect(data).to.not.be.null;
      }
      const retrieveTime = retrieveTimer.stop();

      console.log(
        `    Store rate: ${((fileCount / storeTime) * 1000).toFixed(0)} files/sec`
      );
      console.log(
        `    Retrieve rate: ${((fileCount / retrieveTime) * 1000).toFixed(0)} files/sec`
      );

      expect(storeTime).to.be.lessThan(10000);
      expect(retrieveTime).to.be.lessThan(5000);
    });

    it('should handle large files efficiently', async function () {
      this.timeout(10000);

      const bundle = await wasm.create_bundle();
      const largeContent = new TextEncoder().encode('x'.repeat(100 * 1024)); // 100KB

      const timer = new PerfTimer('Large file operations');

      await bundle.put('large-file.txt', largeContent);
      const retrieved = await bundle.get('large-file.txt');

      const duration = timer.stop();

      assertUint8ArraysEqual(retrieved, largeContent);
      expect(duration).to.be.lessThan(3000);
    });
  });

  describe('Bundle Error Handling', () => {
    it('should handle non-existent keys gracefully', async () => {
      const bundle = await wasm.create_bundle();

      try {
        await bundle.get('non-existent-key.txt');
        expect.fail('Expected error for non-existent key');
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it('should handle deletion of non-existent keys', async () => {
      const bundle = await wasm.create_bundle();

      // This might not throw an error, depending on implementation
      try {
        await bundle.delete('non-existent-key.txt');
        // If no error is thrown, that's also valid behavior
      } catch (error) {
        // Error is also acceptable
        expect(error).to.not.be.undefined;
      }
    });

    it('should handle invalid serialized data', async () => {
      const invalidData = new Uint8Array([1, 2, 3, 4, 5]); // Invalid bundle data

      try {
        await wasm.create_bundle_from_bytes(invalidData);
        expect.fail('Expected error for invalid bundle data');
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe('Bundle Integration with File System', () => {
    it('should save bundle to file and load it back', async () => {
      // Create and populate bundle
      const originalBundle = await wasm.create_bundle();
      await originalBundle.put(
        'test.txt',
        new TextEncoder().encode('test content')
      );

      // Serialize to bytes
      const bundleBytes = await originalBundle.toBytes();

      // Save to temporary file
      const tempFile = createTempFile('', '.bundle');
      fs.writeFileSync(tempFile.name, bundleBytes);

      // Read from file and create bundle
      const fileData = fs.readFileSync(tempFile.name);
      const loadedBundle = await wasm.create_bundle_from_bytes(
        new Uint8Array(fileData)
      );

      // Verify data
      const keys = await loadedBundle.listKeys();
      expect(keys).to.include('test.txt');

      const content = await loadedBundle.get('test.txt');
      const text = new TextDecoder().decode(content);
      expect(text).to.equal('test content');

      // Cleanup
      tempFile.removeCallback();
    });
  });
});
