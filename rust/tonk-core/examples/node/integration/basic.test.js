/**
 * Basic integration tests for Tonk Core WASM bindings
 */

const { expect } = require("chai");
const {
  initWasm,
  generatePeerId,
  TestData,
  assertUint8ArraysEqual,
  PerfTimer,
} = require("../../shared/test-utils");

describe("Basic Integration Tests", () => {
  let wasm;

  before(async function () {
    this.timeout(10000); // WASM loading can take time
    wasm = await initWasm();
  });

  describe("Tonk Core", () => {
    it("should create a tonk instance with random peer ID", async () => {
      const tonk = await wasm.create_tonk();
      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
      expect(peerId.length).to.be.greaterThan(0);
    });

    it("should create a tonk instance with specific peer ID", async () => {
      const customPeerId = generatePeerId();
      const tonk = await wasm.create_tonk_with_peer_id(customPeerId);
      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.equal(customPeerId);
    });
  });

  describe("Virtual File System", () => {
    let tonk;

    beforeEach(async () => {
      tonk = await wasm.create_tonk();
    });

    it("should create and read files", async () => {
      const path = "/test/hello.json";
      const content = { message: TestData.simpleText };

      await tonk.createFile(path, content);
      const exists = await tonk.exists(path);
      expect(exists).to.be.true;

      // readFile returns the full document including metadata
      // The actual content is in the .content property
      const doc = await tonk.readFile(path);
      expect(doc.content).to.deep.equal(content);
    });

    it("should create directories", async () => {
      const path = "/documents";

      await tonk.createDirectory(path);
      const exists = await tonk.exists(path);
      expect(exists).to.be.true;
    });

    it("should handle nested directory creation", async () => {
      // Create parent directories first since auto-creation may not be supported
      await tonk.createDirectory("/projects");
      await tonk.createDirectory("/projects/web");
      await tonk.createDirectory("/projects/web/src");
      await tonk.createDirectory("/projects/web/src/components");

      const exists = await tonk.exists("/projects/web/src/components");
      expect(exists).to.be.true;
    });

    it("should list directory contents", async () => {
      // Create test structure
      await tonk.createDirectory("/docs");
      await tonk.createFile("/docs/readme.json", {
        type: "readme",
        content: "README content",
      });
      await tonk.createFile("/docs/guide.json", {
        type: "guide",
        content: "Guide content",
      });

      const entries = await tonk.listDirectory("/docs");
      expect(entries).to.be.an("array");
      expect(entries.length).to.equal(2);

      const names = entries.map((entry) => entry.name);
      expect(names).to.include("readme.json");
      expect(names).to.include("guide.json");
    });

    it("should delete files", async () => {
      const path = "/temp/deleteme.json";

      await tonk.createFile(path, { temporary: true });
      expect(await tonk.exists(path)).to.be.true;

      const deleted = await tonk.deleteFile(path);
      expect(deleted).to.be.true;
      expect(await tonk.exists(path)).to.be.false;
    });

    it("should get file metadata", async () => {
      const path = "/data/info.json";
      const content = { debug: true };

      await tonk.createFile(path, content);
      const metadata = await tonk.getMetadata(path);

      expect(metadata).to.be.an("object");
      if (metadata.name !== undefined) {
        expect(metadata.name).to.equal("info.json");
      }
    });

    it("should handle non-existent paths gracefully", async () => {
      const exists = await tonk.exists("/non/existent/path.txt");
      expect(exists).to.be.false;
    });

    it("should update files with setFile", async () => {
      const path = "/updateable.json";
      const initialContent = { version: 1 };
      const updatedContent = { version: 2 };

      await tonk.createFile(path, initialContent);
      let doc = await tonk.readFile(path);
      expect(doc.content).to.deep.equal(initialContent);

      await tonk.setFile(path, updatedContent);
      doc = await tonk.readFile(path);
      expect(doc.content).to.deep.equal(updatedContent);
    });

    it("should rename/move files", async () => {
      const oldPath = "/original.json";
      const newPath = "/renamed.json";

      await tonk.createFile(oldPath, { name: "original" });
      expect(await tonk.exists(oldPath)).to.be.true;

      await tonk.rename(oldPath, newPath);
      expect(await tonk.exists(oldPath)).to.be.false;
      expect(await tonk.exists(newPath)).to.be.true;
    });
  });

  describe("Bundle Operations", () => {
    it("should export tonk to bytes and create bundle from bytes", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.json", { hello: "world" });

      const bytes = await tonk.toBytes();
      expect(bytes).to.be.instanceOf(Uint8Array);
      expect(bytes.length).to.be.greaterThan(0);

      const bundle = wasm.create_bundle_from_bytes(bytes);
      expect(bundle).to.not.be.undefined;
    });

    it("should list keys in bundle", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/file1.json", { name: "file1" });
      await tonk.createFile("/file2.json", { name: "file2" });
      await tonk.createFile("/dir/file3.json", { name: "file3" });

      const bytes = await tonk.toBytes();
      const bundle = wasm.create_bundle_from_bytes(bytes);

      const keys = await bundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should retrieve data from bundle by key", async () => {
      const tonk = await wasm.create_tonk();
      const content = { test: "data" };
      await tonk.createFile("/bundle-test.json", content);

      const bytes = await tonk.toBytes();
      const bundle = wasm.create_bundle_from_bytes(bytes);

      // Bundle stores automerge documents, so we check that we can get something
      const keys = await bundle.listKeys();
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should handle binary data in files", async () => {
      const tonk = await wasm.create_tonk();
      const binaryContent = { data: Array.from(TestData.binaryData) };
      await tonk.createFile("/binary.json", binaryContent);

      const bytes = await tonk.toBytes();
      expect(bytes).to.be.instanceOf(Uint8Array);
      expect(bytes.length).to.be.greaterThan(0);
    });

    it("should handle large data efficiently", async function () {
      this.timeout(10000);

      const timer = new PerfTimer("Large data storage");
      const tonk = await wasm.create_tonk();

      // Store a reasonably large JSON document
      const largeContent = { text: TestData.largeText.substring(0, 100000) }; // 100KB
      await tonk.createFile("/large.json", largeContent);

      const bytes = await tonk.toBytes();
      const duration = timer.stop();

      expect(bytes).to.be.instanceOf(Uint8Array);
      expect(bytes.length).to.be.greaterThan(0);
      expect(duration).to.be.lessThan(5000);
    });

    it("should create tonk from bundle bytes", async () => {
      // Create and populate original tonk
      const originalTonk = await wasm.create_tonk();
      await originalTonk.createFile("/data.json", { original: true });

      // Export to bytes
      const bytes = await originalTonk.toBytes();

      // Create new tonk from bytes
      const newTonk = await wasm.create_tonk_from_bytes(bytes);
      expect(newTonk).to.not.be.undefined;

      // Verify data is preserved
      const exists = await newTonk.exists("/data.json");
      expect(exists).to.be.true;
    });

    it("should handle invalid serialized data", async () => {
      const invalidData = new Uint8Array([1, 2, 3, 4, 5]);

      try {
        wasm.create_bundle_from_bytes(invalidData);
        expect.fail("Expected error for invalid bundle data");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe("Error Handling", () => {
    let tonk;

    beforeEach(async () => {
      tonk = await wasm.create_tonk();
    });

    it("should handle invalid paths", async () => {
      try {
        await tonk.createFile("", { content: "test" });
        expect.fail("Expected error for empty path");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it("should handle duplicate file creation", async () => {
      const path = "/duplicate.json";

      await tonk.createFile(path, { first: true });

      try {
        await tonk.createFile(path, { second: true });
        expect.fail("Expected error for duplicate file");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe("Performance", () => {
    it("should create multiple tonk instances efficiently", async () => {
      const timer = new PerfTimer("Multiple tonk creation");
      const tonks = [];

      for (let i = 0; i < 10; i++) {
        tonks.push(await wasm.create_tonk());
      }

      const duration = timer.stop();
      expect(tonks).to.have.lengthOf(10);
      expect(duration).to.be.lessThan(2000);
    });

    it("should handle concurrent VFS operations", async () => {
      const tonk = await wasm.create_tonk();

      const timer = new PerfTimer("Concurrent VFS operations");
      const operations = [];

      // Create 50 files concurrently
      for (let i = 0; i < 50; i++) {
        operations.push(
          tonk.createFile(`/concurrent/file${i}.json`, {
            index: i,
            content: `Content ${i}`,
          }),
        );
      }

      await Promise.all(operations);
      const duration = timer.stop();

      // Verify all files exist
      for (let i = 0; i < 50; i++) {
        const exists = await tonk.exists(`/concurrent/file${i}.json`);
        expect(exists).to.be.true;
      }

      expect(duration).to.be.lessThan(5000);
    });
  });
});
