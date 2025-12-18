/**
 * Sync engine integration tests
 */

const { expect } = require('chai');
const {
  initWasm,
  generatePeerId,
  waitFor,
  sleep,
  PerfTimer,
} = require('../../shared/test-utils');

describe('Sync Engine Integration Tests', () => {
  let wasm;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  describe('Engine Lifecycle', () => {
    it('should create multiple engines with unique peer IDs', async () => {
      const engines = [];
      const peerIds = new Set();

      // Create 5 engines
      for (let i = 0; i < 5; i++) {
        const engine = await wasm.create_sync_engine();
        const peerId = await engine.getPeerId();

        engines.push(engine);
        peerIds.add(peerId);
      }

      // All peer IDs should be unique
      expect(peerIds.size).to.equal(5);
      expect(engines).to.have.lengthOf(5);
    });

    it('should create engines with custom peer IDs', async () => {
      const customIds = ['peer-alpha', 'peer-beta', 'peer-gamma'];
      const engines = [];

      for (const customId of customIds) {
        const engine = await wasm.create_sync_engine_with_peer_id(customId);
        const peerId = await engine.getPeerId();

        expect(peerId).to.equal(customId);
        engines.push(engine);
      }

      expect(engines).to.have.lengthOf(customIds.length);
    });

    it('should handle rapid engine creation', async () => {
      const timer = new PerfTimer('Rapid engine creation');
      const promises = [];

      // Create 20 engines concurrently
      for (let i = 0; i < 20; i++) {
        promises.push(wasm.create_sync_engine());
      }

      const engines = await Promise.all(promises);
      const duration = timer.stop();

      expect(engines).to.have.lengthOf(20);
      expect(duration).to.be.lessThan(5000);

      // Verify all engines have unique peer IDs
      const peerIds = await Promise.all(
        engines.map(engine => engine.getPeerId())
      );
      const uniqueIds = new Set(peerIds);
      expect(uniqueIds.size).to.equal(20);
    });
  });

  describe('VFS Integration', () => {
    let engine, vfs;

    beforeEach(async () => {
      engine = await wasm.create_sync_engine();
      vfs = await engine.getVfs();
    });

    it('should maintain VFS state across operations', async () => {
      // Create a complex directory structure
      const structure = [
        { type: 'dir', path: '/projects' },
        { type: 'dir', path: '/projects/web-app' },
        { type: 'dir', path: '/projects/web-app/src' },
        {
          type: 'file',
          path: '/projects/web-app/src/index.js',
          content: 'console.log("Hello");',
        },
        {
          type: 'file',
          path: '/projects/web-app/package.json',
          content: '{"name": "web-app"}',
        },
        { type: 'dir', path: '/projects/mobile-app' },
        {
          type: 'file',
          path: '/projects/mobile-app/main.dart',
          content: 'void main() {}',
        },
      ];

      // Create structure
      for (const item of structure) {
        if (item.type === 'dir') {
          await vfs.createDirectory(item.path);
        } else {
          await vfs.createFile(item.path, item.content);
        }
      }

      // Verify all items exist
      for (const item of structure) {
        const exists = await vfs.exists(item.path);
        expect(exists).to.be.true;
      }

      // Test directory listing
      const webAppContents = await vfs.listDirectory('/projects/web-app');
      expect(webAppContents).to.have.lengthOf(2); // src directory and package.json

      const srcContents = await vfs.listDirectory('/projects/web-app/src');
      expect(srcContents).to.have.lengthOf(1); // index.js
    });

    it('should handle concurrent VFS operations safely', async function () {
      this.timeout(10000);

      const operations = [];

      // Perform many concurrent operations
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.txt`;

        operations.push(
          vfs
            .createDirectory(dirPath)
            .then(() => vfs.createFile(filePath, `Content ${i}`))
        );
      }

      await Promise.all(operations);

      // Verify all operations completed successfully
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.txt`;

        expect(await vfs.exists(dirPath)).to.be.true;
        expect(await vfs.exists(filePath)).to.be.true;
      }
    });

    it('should persist VFS changes across VFS instances', async () => {
      // Create files with first VFS instance
      await vfs.createDirectory('/persistent');
      await vfs.createFile('/persistent/test.txt', 'persistent data');

      // Get a new VFS instance from the same engine
      const vfs2 = await engine.getVfs();

      // Verify data is accessible from second instance
      const exists = await vfs2.exists('/persistent/test.txt');
      expect(exists).to.be.true;

      const entries = await vfs2.listDirectory('/persistent');
      expect(entries).to.have.lengthOf(1);
      expect(entries[0].name).to.equal('test.txt');
    });
  });

  describe('Document Management', () => {
    let engine;

    beforeEach(async () => {
      engine = await wasm.create_sync_engine();
    });

    // Note: These tests assume WASM bindings for document operations exist
    // They may need to be adjusted based on actual implementation

    it('should create and manage documents', async () => {
      // This test would require WASM bindings for document creation
      // For now, we'll test what we can access

      const peerId = await engine.getPeerId();
      expect(peerId).to.be.a('string');

      // Future: Test document creation, retrieval, and modification
      // const doc = await engine.createDocument();
      // const docId = await doc.getId();
      // expect(docId).to.be.a('string');
    });
  });

  describe('Memory Management', () => {
    it('should handle engine cleanup properly', async () => {
      const engines = [];

      // Create many engines
      for (let i = 0; i < 100; i++) {
        engines.push(await wasm.create_sync_engine());
      }

      // Get peer IDs to ensure engines are working
      const peerIds = await Promise.all(
        engines.map(engine => engine.getPeerId())
      );

      expect(peerIds).to.have.lengthOf(100);
      expect(new Set(peerIds).size).to.equal(100); // All unique

      // Note: In a real scenario, we'd want to test that memory is properly freed
      // This would require additional WASM bindings or monitoring
    });

    it('should handle VFS operations under memory pressure', async function () {
      this.timeout(15000);

      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();

      // Create many files to simulate memory pressure
      const fileCount = 500;
      for (let i = 0; i < fileCount; i++) {
        const path = `/memory-test/file-${i}.txt`;
        const content = `Content for file ${i} - ${'x'.repeat(100)}`;
        await vfs.createFile(path, content);

        // Periodically check that we can still list directories
        if (i % 100 === 0) {
          const entries = await vfs.listDirectory('/memory-test');
          expect(entries.length).to.be.greaterThan(0);
        }
      }

      // Final verification
      const finalEntries = await vfs.listDirectory('/memory-test');
      expect(finalEntries).to.have.lengthOf(fileCount);
    });
  });

  describe('Error Recovery', () => {
    it('should recover from VFS errors gracefully', async () => {
      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();

      // Try to create a file with invalid path
      try {
        await vfs.createFile('', 'content');
        expect.fail('Expected error for empty path');
      } catch (error) {
        // Error is expected
        expect(error).to.not.be.undefined;
      }

      // Verify VFS still works after error
      await vfs.createFile('/recovery-test.txt', 'recovery content');
      const exists = await vfs.exists('/recovery-test.txt');
      expect(exists).to.be.true;
    });

    it('should handle engine state after errors', async () => {
      const engine = await wasm.create_sync_engine();

      // Engine should still be functional after VFS errors
      const vfs1 = await engine.getVfs();

      try {
        await vfs1.createFile('', 'bad content');
      } catch (error) {
        // Expected
      }

      // Should still be able to get new VFS instance
      const vfs2 = await engine.getVfs();
      await vfs2.createFile('/post-error.txt', 'content');

      const exists = await vfs2.exists('/post-error.txt');
      expect(exists).to.be.true;
    });
  });

  describe('Performance Benchmarks', () => {
    it('should benchmark engine creation performance', async () => {
      const iterations = 50;
      const times = [];

      for (let i = 0; i < iterations; i++) {
        const timer = new PerfTimer();
        await wasm.create_sync_engine();
        times.push(timer.stop());
      }

      const avgTime = times.reduce((a, b) => a + b) / times.length;
      const minTime = Math.min(...times);
      const maxTime = Math.max(...times);

      console.log(`    Engine creation stats (${iterations} iterations):`);
      console.log(`      Average: ${avgTime.toFixed(2)}ms`);
      console.log(`      Min: ${minTime.toFixed(2)}ms`);
      console.log(`      Max: ${maxTime.toFixed(2)}ms`);

      expect(avgTime).to.be.lessThan(100); // Should average under 100ms
    });

    it('should benchmark VFS operation performance', async () => {
      const engine = await wasm.create_sync_engine();
      const vfs = await engine.getVfs();

      const operations = ['createFile', 'exists', 'listDirectory'];
      const benchmarks = {};

      // Benchmark file creation
      const createTimer = new PerfTimer();
      for (let i = 0; i < 100; i++) {
        await vfs.createFile(`/bench/file${i}.txt`, `content ${i}`);
      }
      benchmarks.createFile = createTimer.stop();

      // Benchmark exists checks
      const existsTimer = new PerfTimer();
      for (let i = 0; i < 100; i++) {
        await vfs.exists(`/bench/file${i}.txt`);
      }
      benchmarks.exists = existsTimer.stop();

      // Benchmark directory listings
      const listTimer = new PerfTimer();
      for (let i = 0; i < 20; i++) {
        await vfs.listDirectory('/bench');
      }
      benchmarks.listDirectory = listTimer.stop();

      console.log(`    VFS benchmarks:`);
      console.log(
        `      File creation (100 ops): ${benchmarks.createFile.toFixed(2)}ms`
      );
      console.log(
        `      Exists checks (100 ops): ${benchmarks.exists.toFixed(2)}ms`
      );
      console.log(
        `      Directory listings (20 ops): ${benchmarks.listDirectory.toFixed(2)}ms`
      );

      // All operations should complete reasonably quickly
      expect(benchmarks.createFile).to.be.lessThan(5000);
      expect(benchmarks.exists).to.be.lessThan(1000);
      expect(benchmarks.listDirectory).to.be.lessThan(1000);
    });
  });
});
