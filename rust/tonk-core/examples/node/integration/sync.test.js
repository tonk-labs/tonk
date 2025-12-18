/**
 * TonkCore and VFS integration tests
 */

const { expect } = require("chai");
const {
  initWasm,
  generatePeerId,
  waitFor,
  sleep,
  PerfTimer,
} = require("../../shared/test-utils");

describe("TonkCore Integration Tests", () => {
  let wasm;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  describe("TonkCore Lifecycle", () => {
    it("should create multiple TonkCore instances with unique peer IDs", async () => {
      const instances = [];
      const peerIds = new Set();

      // Create 5 instances
      for (let i = 0; i < 5; i++) {
        const tonk = await wasm.create_tonk();
        const peerId = await tonk.getPeerId();

        instances.push(tonk);
        peerIds.add(peerId);
      }

      // All peer IDs should be unique
      expect(peerIds.size).to.equal(5);
      expect(instances).to.have.lengthOf(5);
    });

    it("should create TonkCore instances with custom peer IDs", async () => {
      const customIds = ["peer-alpha", "peer-beta", "peer-gamma"];
      const instances = [];

      for (const customId of customIds) {
        const tonk = await wasm.create_tonk_with_peer_id(customId);
        const peerId = await tonk.getPeerId();

        expect(peerId).to.equal(customId);
        instances.push(tonk);
      }

      expect(instances).to.have.lengthOf(customIds.length);
    });

    it("should handle rapid TonkCore creation", async () => {
      const timer = new PerfTimer("Rapid TonkCore creation");
      const promises = [];

      // Create 20 instances concurrently
      for (let i = 0; i < 20; i++) {
        promises.push(wasm.create_tonk());
      }

      const instances = await Promise.all(promises);
      const duration = timer.stop();

      expect(instances).to.have.lengthOf(20);
      expect(duration).to.be.lessThan(5000);

      // Verify all instances have unique peer IDs
      const peerIds = await Promise.all(
        instances.map((tonk) => tonk.getPeerId()),
      );
      const uniqueIds = new Set(peerIds);
      expect(uniqueIds.size).to.equal(20);
    });

    it("should create TonkCore with storage configuration", async () => {
      // Test in-memory storage
      const tonk = await wasm.create_tonk_with_storage(false, null);
      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
    });

    it("should create TonkCore with full configuration", async () => {
      const customPeerId = "configured-peer";
      const tonk = await wasm.create_tonk_with_config(customPeerId, false, null);

      const peerId = await tonk.getPeerId();
      expect(peerId).to.equal(customPeerId);
    });
  });

  describe("VFS Operations", () => {
    let tonk;

    beforeEach(async () => {
      tonk = await wasm.create_tonk();
    });

    it("should maintain VFS state across operations", async () => {
      // Create a complex directory structure
      const structure = [
        { type: "dir", path: "/projects" },
        { type: "dir", path: "/projects/web-app" },
        { type: "dir", path: "/projects/web-app/src" },
        {
          type: "file",
          path: "/projects/web-app/src/index.js",
          content: 'console.log("Hello");',
        },
        {
          type: "file",
          path: "/projects/web-app/package.json",
          content: { name: "web-app" },
        },
        { type: "dir", path: "/projects/mobile-app" },
        {
          type: "file",
          path: "/projects/mobile-app/main.dart",
          content: "void main() {}",
        },
      ];

      // Create structure
      for (const item of structure) {
        if (item.type === "dir") {
          await tonk.createDirectory(item.path);
        } else {
          await tonk.createFile(item.path, item.content);
        }
      }

      // Verify all items exist
      for (const item of structure) {
        const exists = await tonk.exists(item.path);
        expect(exists).to.be.true;
      }

      // Test directory listing
      const webAppContents = await tonk.listDirectory("/projects/web-app");
      expect(webAppContents).to.have.lengthOf(2); // src directory and package.json

      const srcContents = await tonk.listDirectory("/projects/web-app/src");
      expect(srcContents).to.have.lengthOf(1); // index.js
    });

    it("should handle concurrent VFS operations safely", async function () {
      this.timeout(10000);

      // First create the parent directory
      await tonk.createDirectory("/concurrent");

      const operations = [];

      // Perform many concurrent operations
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.txt`;

        operations.push(
          tonk
            .createDirectory(dirPath)
            .then(() => tonk.createFile(filePath, `Content ${i}`)),
        );
      }

      await Promise.all(operations);

      // Verify all operations completed successfully
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.txt`;

        expect(await tonk.exists(dirPath)).to.be.true;
        expect(await tonk.exists(filePath)).to.be.true;
      }
    });

    it("should persist VFS changes through export/import", async () => {
      // Create files
      await tonk.createDirectory("/persistent");
      await tonk.createFile("/persistent/test.txt", "persistent data");

      // Export to bytes
      const bytes = await tonk.toBytes(null);

      // Create new TonkCore from bytes
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      // Verify data is accessible
      const exists = await tonk2.exists("/persistent/test.txt");
      expect(exists).to.be.true;

      const entries = await tonk2.listDirectory("/persistent");
      expect(entries).to.have.lengthOf(1);
      expect(entries[0].name).to.equal("test.txt");
    });
  });

  describe("Document Management", () => {
    let tonk;

    beforeEach(async () => {
      tonk = await wasm.create_tonk();
    });

    it("should create and read documents with various content types", async () => {
      const testCases = [
        { path: "/string.txt", content: "Simple string" },
        { path: "/number.json", content: 42 },
        { path: "/object.json", content: { key: "value", nested: { a: 1 } } },
        { path: "/array.json", content: [1, 2, 3, "four"] },
        { path: "/boolean.json", content: true },
      ];

      for (const testCase of testCases) {
        await tonk.createFile(testCase.path, testCase.content);
        const retrieved = await tonk.readFile(testCase.path);
        // Handle both wrapped primitives ({ value: ... }) and objects
        const actualContent = retrieved.content.value !== undefined
          ? retrieved.content.value
          : retrieved.content;
        expect(actualContent).to.deep.equal(testCase.content);
      }
    });

    it("should update document content", async () => {
      await tonk.createFile("/updatable.json", { version: 1 });

      // Update content
      await tonk.setFile("/updatable.json", { version: 2, updated: true });

      const retrieved = await tonk.readFile("/updatable.json");
      expect(retrieved.content).to.deep.equal({ version: 2, updated: true });
    });

    it("should handle document metadata", async () => {
      await tonk.createFile("/with-metadata.txt", "Content");

      const metadata = await tonk.getMetadata("/with-metadata.txt");
      expect(metadata).to.be.an("object");

      if (metadata.timestamps) {
        expect(metadata.timestamps).to.have.property("created");
        expect(metadata.timestamps).to.have.property("modified");
      }
    });
  });

  describe("Memory and Resource Management", () => {
    it("should handle many TonkCore instances", async () => {
      const instances = [];

      // Create many instances
      for (let i = 0; i < 100; i++) {
        instances.push(await wasm.create_tonk());
      }

      // Get peer IDs to ensure instances are working
      const peerIds = await Promise.all(
        instances.map((tonk) => tonk.getPeerId()),
      );

      expect(peerIds).to.have.lengthOf(100);
      expect(new Set(peerIds).size).to.equal(100); // All unique
    });

    it("should handle VFS operations under load", async function () {
      this.timeout(20000);

      const tonk = await wasm.create_tonk();

      // Create many files to simulate load
      const fileCount = 500;
      for (let i = 0; i < fileCount; i++) {
        const path = `/load-test/file-${i}.txt`;
        const content = `Content for file ${i} - ${"x".repeat(100)}`;
        await tonk.createFile(path, content);

        // Periodically check that we can still list directories
        if (i % 100 === 0) {
          const entries = await tonk.listDirectory("/load-test");
          expect(entries.length).to.be.greaterThan(0);
        }
      }

      // Final verification
      const finalEntries = await tonk.listDirectory("/load-test");
      expect(finalEntries).to.have.lengthOf(fileCount);
    });
  });

  describe("Error Recovery", () => {
    it("should recover from VFS errors gracefully", async () => {
      const tonk = await wasm.create_tonk();

      // Try to create a file with invalid path
      try {
        await tonk.createFile("", "content");
        expect.fail("Expected error for empty path");
      } catch (error) {
        // Error is expected
        expect(error).to.not.be.undefined;
      }

      // Verify VFS still works after error
      await tonk.createFile("/recovery-test.txt", "recovery content");
      const exists = await tonk.exists("/recovery-test.txt");
      expect(exists).to.be.true;
    });

    it("should handle TonkCore state after errors", async () => {
      const tonk = await wasm.create_tonk();

      // Cause an error
      try {
        await tonk.createFile("", "bad content");
      } catch (error) {
        // Expected
      }

      // Should still be able to create files
      await tonk.createFile("/post-error.txt", "content");
      const exists = await tonk.exists("/post-error.txt");
      expect(exists).to.be.true;

      // Peer ID should still be accessible
      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
    });

    it("should handle duplicate file errors", async () => {
      const tonk = await wasm.create_tonk();

      await tonk.createFile("/original.txt", "original");

      try {
        await tonk.createFile("/original.txt", "duplicate");
        expect.fail("Expected error for duplicate file");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }

      // Original should still be accessible
      const content = await tonk.readFile("/original.txt");
      // Handle wrapped primitive
      const actualContent = content.content.value !== undefined
        ? content.content.value
        : content.content;
      expect(actualContent).to.equal("original");
    });
  });

  describe("Performance Benchmarks", () => {
    it("should benchmark TonkCore creation performance", async () => {
      const iterations = 50;
      const times = [];

      for (let i = 0; i < iterations; i++) {
        const timer = new PerfTimer();
        await wasm.create_tonk();
        times.push(timer.stop());
      }

      const avgTime = times.reduce((a, b) => a + b) / times.length;
      const minTime = Math.min(...times);
      const maxTime = Math.max(...times);

      console.log(`    TonkCore creation stats (${iterations} iterations):`);
      console.log(`      Average: ${avgTime.toFixed(2)}ms`);
      console.log(`      Min: ${minTime.toFixed(2)}ms`);
      console.log(`      Max: ${maxTime.toFixed(2)}ms`);

      expect(avgTime).to.be.lessThan(100); // Should average under 100ms
    });

    it("should benchmark VFS operation performance", async () => {
      const tonk = await wasm.create_tonk();
      const benchmarks = {};

      // Benchmark file creation
      const createTimer = new PerfTimer();
      for (let i = 0; i < 100; i++) {
        await tonk.createFile(`/bench/file${i}.txt`, `content ${i}`);
      }
      benchmarks.createFile = createTimer.stop();

      // Benchmark exists checks
      const existsTimer = new PerfTimer();
      for (let i = 0; i < 100; i++) {
        await tonk.exists(`/bench/file${i}.txt`);
      }
      benchmarks.exists = existsTimer.stop();

      // Benchmark directory listings
      const listTimer = new PerfTimer();
      for (let i = 0; i < 20; i++) {
        await tonk.listDirectory("/bench");
      }
      benchmarks.listDirectory = listTimer.stop();

      console.log(`    VFS benchmarks:`);
      console.log(
        `      File creation (100 ops): ${benchmarks.createFile.toFixed(2)}ms`,
      );
      console.log(
        `      Exists checks (100 ops): ${benchmarks.exists.toFixed(2)}ms`,
      );
      console.log(
        `      Directory listings (20 ops): ${benchmarks.listDirectory.toFixed(2)}ms`,
      );

      // All operations should complete reasonably quickly
      expect(benchmarks.createFile).to.be.lessThan(5000);
      expect(benchmarks.exists).to.be.lessThan(1000);
      expect(benchmarks.listDirectory).to.be.lessThan(1000);
    });

    it("should benchmark export/import performance", async () => {
      const tonk = await wasm.create_tonk();

      // Create test data
      for (let i = 0; i < 100; i++) {
        await tonk.createFile(`/export-test/file${i}.txt`, `content ${i}`);
      }

      // Benchmark export
      const exportTimer = new PerfTimer();
      const bytes = await tonk.toBytes(null);
      const exportTime = exportTimer.stop();

      // Benchmark import
      const importTimer = new PerfTimer();
      await wasm.create_tonk_from_bytes(bytes);
      const importTime = importTimer.stop();

      console.log(`    Export/Import benchmarks:`);
      console.log(`      Export (100 files): ${exportTime.toFixed(2)}ms`);
      console.log(`      Import (100 files): ${importTime.toFixed(2)}ms`);

      expect(exportTime).to.be.lessThan(3000);
      expect(importTime).to.be.lessThan(3000);
    });
  });
});
