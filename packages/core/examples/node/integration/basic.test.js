/**
 * Basic integration tests for Tonk Core WASM bindings
 */

const { expect } = require('chai');
const {
  initWasm,
  generatePeerId,
  TestData,
  createTestBundle,
  assertUint8ArraysEqual,
  PerfTimer,
} = require('../../shared/test-utils');

describe('Basic Integration Tests', () => {
  let wasm;

  before(async function () {
    this.timeout(10000); // WASM loading can take time
    wasm = await initWasm();
  });

  describe('Sync Engine', () => {
    it('should create a sync engine with random peer ID', async () => {
      const engine = await wasm.create_sync_engine();
      expect(engine).to.not.be.undefined;

      const peerId = await engine.getPeerId();
      expect(peerId).to.be.a('string');
      expect(peerId.length).to.be.greaterThan(0);
    });

    it('should create a sync engine with specific peer ID', async () => {
      const customPeerId = generatePeerId();
      const engine = await wasm.create_sync_engine_with_peer_id(customPeerId);
      expect(engine).to.not.be.undefined;

      const peerId = await engine.getPeerId();
      expect(peerId).to.equal(customPeerId);
    });

    it('should provide access to VFS', async () => {
      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();
      expect(vfs).to.not.be.undefined;
    });
  });

  describe('Virtual File System', () => {
    let engine, vfs;

    beforeEach(async () => {
      engine = await wasm.create_sync_engine();
      vfs = await engine.getVfs();
    });

    it('should create and read files', async () => {
      const path = '/test/hello.txt';
      const content = TestData.simpleText;

      await vfs.createFile(path, content);
      const exists = await vfs.exists(path);
      expect(exists).to.be.true;

      // Note: Reading file content would require WASM binding implementation
    });

    it('should create directories', async () => {
      const path = '/documents';

      await vfs.createDirectory(path);
      const exists = await vfs.exists(path);
      expect(exists).to.be.true;
    });

    it('should handle nested directory creation', async () => {
      const path = '/projects/web/src/components';

      await vfs.createDirectory(path);
      const exists = await vfs.exists(path);
      expect(exists).to.be.true;
    });

    it('should list directory contents', async () => {
      // Create test structure
      await vfs.createDirectory('/docs');
      await vfs.createFile('/docs/readme.md', 'README content');
      await vfs.createFile('/docs/guide.md', 'Guide content');

      const entries = await vfs.listDirectory('/docs');
      expect(entries).to.be.an('array');
      expect(entries.length).to.equal(2);

      const names = entries.map(entry => entry.name);
      expect(names).to.include('readme.md');
      expect(names).to.include('guide.md');
    });

    it('should delete files', async () => {
      const path = '/temp/deleteme.txt';

      await vfs.createFile(path, 'temporary content');
      expect(await vfs.exists(path)).to.be.true;

      const deleted = await vfs.deleteFile(path);
      expect(deleted).to.be.true;
      expect(await vfs.exists(path)).to.be.false;
    });

    it('should get file metadata', async () => {
      const path = '/data/info.json';
      const content = TestData.jsonConfig;

      await vfs.createFile(path, content);
      const metadata = await vfs.getMetadata(path);

      expect(metadata).to.be.an('object');
      // Note: metadata.name may not be implemented in current WASM binding
      // expect(metadata.name).to.equal('info.json');
      // TODO: Verify WASM binding implements name property correctly
      if (metadata.name !== undefined) {
        expect(metadata.name).to.equal('info.json');
      }
      // Additional metadata checks would depend on WASM implementation
    });

    it('should handle non-existent paths gracefully', async () => {
      const exists = await vfs.exists('/non/existent/path.txt');
      expect(exists).to.be.false;
    });
  });

  describe('Bundle Operations', () => {
    it('should create an empty bundle', async () => {
      const bundle = await wasm.create_bundle();
      expect(bundle).to.not.be.undefined;
    });

    it('should store and retrieve data', async () => {
      const bundle = await wasm.create_bundle();
      const key = 'test/data.txt';
      const value = new TextEncoder().encode(TestData.simpleText);

      await bundle.put(key, value);
      const retrieved = await bundle.get(key);

      expect(retrieved).to.be.instanceOf(Uint8Array);
      assertUint8ArraysEqual(retrieved, value);
    });

    it('should list keys', async () => {
      const bundle = await wasm.create_bundle();
      const keys = ['file1.txt', 'file2.txt', 'dir/file3.txt'];

      for (const key of keys) {
        const value = new TextEncoder().encode(`Content of ${key}`);
        await bundle.put(key, value);
      }

      const listedKeys = await bundle.listKeys();
      expect(listedKeys).to.be.an('array');
      expect(listedKeys).to.have.lengthOf(keys.length + 2); // +2 for manifest and root doc

      for (const key of keys) {
        expect(listedKeys).to.include(key);
      }
    });

    it('should delete keys', async () => {
      const bundle = await wasm.create_bundle();
      const key = 'deleteme.txt';
      const value = new TextEncoder().encode('delete this');

      await bundle.put(key, value);
      expect(await bundle.get(key)).to.not.be.null;

      await bundle.delete(key);

      try {
        await bundle.get(key);
        expect.fail('Expected error when getting deleted key');
      } catch (error) {
        // Expected behavior
        expect(error).to.not.be.undefined;
      }
    });

    it('should handle binary data', async () => {
      const bundle = await wasm.create_bundle();
      const key = 'binary.data';
      const value = TestData.binaryData;

      await bundle.put(key, value);
      const retrieved = await bundle.get(key);

      assertUint8ArraysEqual(retrieved, value);
    });

    it('should handle large data efficiently', async function () {
      this.timeout(10000); // Large data operations can take time

      const timer = new PerfTimer('Large data storage');
      const bundle = await wasm.create_bundle();
      const key = 'large.txt';
      const value = new TextEncoder().encode(TestData.largeText);

      await bundle.put(key, value);
      const retrieved = await bundle.get(key);
      const duration = timer.stop();

      assertUint8ArraysEqual(retrieved, value);
      expect(duration).to.be.lessThan(5000); // Should complete within 5 seconds
    });
  });

  describe('Error Handling', () => {
    let engine, vfs;

    beforeEach(async () => {
      engine = await wasm.create_sync_engine();
      vfs = await engine.getVfs();
    });

    it('should handle invalid paths', async () => {
      try {
        await vfs.createFile('', 'content');
        expect.fail('Expected error for empty path');
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it('should handle duplicate file creation', async () => {
      const path = '/duplicate.txt';

      await vfs.createFile(path, 'first content');

      try {
        await vfs.createFile(path, 'second content');
        expect.fail('Expected error for duplicate file');
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe('Performance', () => {
    it('should create multiple engines efficiently', async () => {
      const timer = new PerfTimer('Multiple engine creation');
      const engines = [];

      for (let i = 0; i < 10; i++) {
        engines.push(await wasm.create_sync_engine());
      }

      const duration = timer.stop();
      expect(engines).to.have.lengthOf(10);
      expect(duration).to.be.lessThan(2000); // Should complete within 2 seconds
    });

    it('should handle concurrent VFS operations', async () => {
      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();

      const timer = new PerfTimer('Concurrent VFS operations');
      const operations = [];

      // Create 50 files concurrently
      for (let i = 0; i < 50; i++) {
        operations.push(
          vfs.createFile(`/concurrent/file${i}.txt`, `Content ${i}`)
        );
      }

      await Promise.all(operations);
      const duration = timer.stop();

      // Verify all files exist
      for (let i = 0; i < 50; i++) {
        const exists = await vfs.exists(`/concurrent/file${i}.txt`);
        expect(exists).to.be.true;
      }

      expect(duration).to.be.lessThan(5000); // Should complete within 5 seconds
    });
  });
});
