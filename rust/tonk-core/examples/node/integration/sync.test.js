/**
 * Tonk Core integration tests
 *
 * Tests the TonkCore WASM API including peer management, VFS operations,
 * and persistence.
 */

const { expect } = require("chai");
const {
  initWasm,
  generatePeerId,
  waitFor,
  sleep,
  PerfTimer,
} = require("../../shared/test-utils");

describe("Tonk Core Integration Tests", () => {
  let wasm;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  describe("Tonk Lifecycle", () => {
    it("should create multiple tonk instances with unique peer IDs", async () => {
      const tonks = [];
      const peerIds = new Set();

      // Create 5 tonk instances
      for (let i = 0; i < 5; i++) {
        const tonk = await wasm.create_tonk();
        const peerId = await tonk.getPeerId();

        tonks.push(tonk);
        peerIds.add(peerId);
      }

      // All peer IDs should be unique
      expect(peerIds.size).to.equal(5);
      expect(tonks).to.have.lengthOf(5);
    });

    it("should create tonk instances with custom peer IDs", async () => {
      const customIds = ["peer-alpha", "peer-beta", "peer-gamma"];
      const tonks = [];

      for (const customId of customIds) {
        const tonk = await wasm.create_tonk_with_peer_id(customId);
        const peerId = await tonk.getPeerId();

        expect(peerId).to.equal(customId);
        tonks.push(tonk);
      }

      expect(tonks).to.have.lengthOf(customIds.length);
    });

    it("should handle rapid tonk creation", async () => {
      const timer = new PerfTimer("Rapid tonk creation");
      const promises = [];

      // Create 20 tonk instances concurrently
      for (let i = 0; i < 20; i++) {
        promises.push(wasm.create_tonk());
      }

      const tonks = await Promise.all(promises);
      const duration = timer.stop();

      expect(tonks).to.have.lengthOf(20);
      expect(duration).to.be.lessThan(5000);

      // Verify all instances have unique peer IDs
      const peerIds = await Promise.all(tonks.map((tonk) => tonk.getPeerId()));
      const uniqueIds = new Set(peerIds);
      expect(uniqueIds.size).to.equal(20);
    });
  });

  describe("VFS Integration", () => {
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
          path: "/projects/web-app/src/index.json",
          content: { code: 'console.log("Hello");' },
        },
        {
          type: "file",
          path: "/projects/web-app/package.json",
          content: { name: "web-app" },
        },
        { type: "dir", path: "/projects/mobile-app" },
        {
          type: "file",
          path: "/projects/mobile-app/main.json",
          content: { code: "void main() {}" },
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
      expect(srcContents).to.have.lengthOf(1); // index.json
    });

    it("should handle concurrent VFS operations safely", async function () {
      this.timeout(10000);

      // Create parent directory first
      await tonk.createDirectory("/concurrent");

      const operations = [];

      // Perform many concurrent operations - create directories first, then files
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.json`;

        operations.push(
          tonk
            .createDirectory(dirPath)
            .then(() => tonk.createFile(filePath, { content: `Content ${i}` })),
        );
      }

      await Promise.all(operations);

      // Verify all operations completed successfully
      for (let i = 0; i < 50; i++) {
        const dirPath = `/concurrent/dir${i}`;
        const filePath = `/concurrent/dir${i}/file${i}.json`;

        expect(await tonk.exists(dirPath)).to.be.true;
        expect(await tonk.exists(filePath)).to.be.true;
      }
    });

    it("should persist VFS changes through export/import cycle", async () => {
      // Create files
      await tonk.createDirectory("/persistent");
      await tonk.createFile("/persistent/test.json", {
        data: "persistent data",
      });

      // Export to bytes
      const bytes = await tonk.toBytes();

      // Create a new tonk from the bytes
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      // Verify data is accessible from the new instance
      const exists = await tonk2.exists("/persistent/test.json");
      expect(exists).to.be.true;

      const entries = await tonk2.listDirectory("/persistent");
      expect(entries).to.have.lengthOf(1);
      expect(entries[0].name).to.equal("test.json");

      // readFile returns full doc with metadata
      const doc = await tonk2.readFile("/persistent/test.json");
      expect(doc.content).to.deep.equal({ data: "persistent data" });
    });
  });

  describe("Document Management", () => {
    let tonk;

    beforeEach(async () => {
      tonk = await wasm.create_tonk();
    });

    it("should create and manage documents", async () => {
      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");

      // Create a document
      await tonk.createFile("/doc.json", {
        title: "Test Document",
        body: "Content here",
      });

      const exists = await tonk.exists("/doc.json");
      expect(exists).to.be.true;

      // readFile returns full doc with metadata
      const doc = await tonk.readFile("/doc.json");
      expect(doc.content.title).to.equal("Test Document");
    });

    it("should update documents with updateFile", async () => {
      const path = "/updatable.json";
      await tonk.createFile(path, { version: 1, items: ["a", "b"] });

      // Update the document
      const updated = await tonk.updateFile(path, {
        version: 2,
        items: ["a", "b", "c"],
      });
      expect(updated).to.be.true;

      // readFile returns full doc with metadata
      const doc = await tonk.readFile(path);
      expect(doc.content.version).to.equal(2);
      expect(doc.content.items).to.deep.equal(["a", "b", "c"]);
    });

    it("should patch documents at specific paths", async () => {
      const path = "/patchable.json";
      await tonk.createFile(path, { config: { theme: "light", size: 12 } });

      // Patch a specific path
      const patched = await tonk.patchFile(path, ["config", "theme"], "dark");
      expect(patched).to.be.true;

      // readFile returns full doc with metadata
      const doc = await tonk.readFile(path);
      expect(doc.content.config.theme).to.equal("dark");
      expect(doc.content.config.size).to.equal(12); // Other fields preserved
    });
  });

  describe("Memory Management", () => {
    it("should handle tonk cleanup properly", async () => {
      const tonks = [];

      // Create many tonk instances
      for (let i = 0; i < 100; i++) {
        tonks.push(await wasm.create_tonk());
      }

      // Get peer IDs to ensure instances are working
      const peerIds = await Promise.all(tonks.map((tonk) => tonk.getPeerId()));

      expect(peerIds).to.have.lengthOf(100);
      expect(new Set(peerIds).size).to.equal(100); // All unique
    });

    it("should handle VFS operations under memory pressure", async function () {
      this.timeout(15000);

      const tonk = await wasm.create_tonk();

      // Create many files to simulate memory pressure
      const fileCount = 500;
      for (let i = 0; i < fileCount; i++) {
        const path = `/memory-test/file-${i}.json`;
        const content = {
          content: `Content for file ${i}`,
          padding: "x".repeat(100),
        };
        await tonk.createFile(path, content);

        // Periodically check that we can still list directories
        if (i % 100 === 0) {
          const entries = await tonk.listDirectory("/memory-test");
          expect(entries.length).to.be.greaterThan(0);
        }
      }

      // Final verification
      const finalEntries = await tonk.listDirectory("/memory-test");
      expect(finalEntries).to.have.lengthOf(fileCount);
    });
  });

  describe("Error Recovery", () => {
    it("should recover from VFS errors gracefully", async () => {
      const tonk = await wasm.create_tonk();

      // Try to create a file with invalid path
      try {
        await tonk.createFile("", { content: "test" });
        expect.fail("Expected error for empty path");
      } catch (error) {
        // Error is expected
        expect(error).to.not.be.undefined;
      }

      // Verify tonk still works after error
      await tonk.createFile("/recovery-test.json", { recovery: true });
      const exists = await tonk.exists("/recovery-test.json");
      expect(exists).to.be.true;
    });

    it("should handle tonk state after errors", async () => {
      const tonk = await wasm.create_tonk();

      try {
        await tonk.createFile("", { bad: "content" });
      } catch (error) {
        // Expected
      }

      // Should still be able to perform operations
      await tonk.createFile("/post-error.json", { content: "test" });

      const exists = await tonk.exists("/post-error.json");
      expect(exists).to.be.true;
    });
  });

  describe("Performance Benchmarks", () => {
    it("should benchmark tonk creation performance", async () => {
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

      console.log(`    Tonk creation stats (${iterations} iterations):`);
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
        await tonk.createFile(`/bench/file${i}.json`, {
          content: `content ${i}`,
        });
      }
      benchmarks.createFile = createTimer.stop();

      // Benchmark exists checks
      const existsTimer = new PerfTimer();
      for (let i = 0; i < 100; i++) {
        await tonk.exists(`/bench/file${i}.json`);
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
  });

  describe("Connection State", () => {
    it("should report connection state", async () => {
      const tonk = await wasm.create_tonk();

      const isConnected = await tonk.isConnected();
      expect(isConnected).to.be.a("boolean");

      const state = await tonk.getConnectionState();
      expect(state).to.be.a("string");
      // Default state should be disconnected
      expect(state).to.equal("disconnected");
    });
  });

  describe("Storage Configuration", () => {
    it("should create tonk with in-memory storage", async () => {
      const tonk = await wasm.create_tonk_with_storage(false, null);
      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");

      // Verify VFS works
      await tonk.createFile("/memory-storage-test.json", { test: true });
      const exists = await tonk.exists("/memory-storage-test.json");
      expect(exists).to.be.true;
    });

    it("should create tonk with full config", async () => {
      const customPeerId = generatePeerId();
      const tonk = await wasm.create_tonk_with_config(
        customPeerId,
        false,
        null,
      );

      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.equal(customPeerId);
    });
  });

  describe("Fork Operations", () => {
    it("should fork tonk to bytes", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/original.json", { original: true });

      // Fork to bytes (creates a copy) - pass undefined/null for config
      const forkedBytes = await tonk.forkToBytes(undefined);
      expect(forkedBytes).to.be.instanceOf(Uint8Array);
      expect(forkedBytes.length).to.be.greaterThan(0);

      // Create new tonk from forked bytes
      const forkedTonk = await wasm.create_tonk_from_bytes(forkedBytes);
      expect(forkedTonk).to.not.be.undefined;

      // Forked tonk should have a valid peer ID
      const forkedPeerId = await forkedTonk.getPeerId();
      expect(forkedPeerId).to.be.a("string");

      // Changes to forked instance shouldn't affect original
      await forkedTonk.createFile("/forked-only.json", { forked: true });
      const forkedHasFile = await forkedTonk.exists("/forked-only.json");
      expect(forkedHasFile).to.be.true;

      const originalHasForked = await tonk.exists("/forked-only.json");
      expect(originalHasForked).to.be.false;
    });

    it("should preserve data through toBytes round-trip", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/preserved.json", { data: "preserved" });

      // Export using toBytes
      const bytes = await tonk.toBytes(undefined);
      expect(bytes).to.be.instanceOf(Uint8Array);

      // Import from bytes
      const restoredTonk = await wasm.create_tonk_from_bytes(bytes);

      // Verify the file exists in restored tonk
      const exists = await restoredTonk.exists("/preserved.json");
      expect(exists).to.be.true;

      // Verify content is preserved
      const doc = await restoredTonk.readFile("/preserved.json");
      expect(doc.content).to.deep.equal({ data: "preserved" });
    });
  });
});
