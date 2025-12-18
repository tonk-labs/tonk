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

  describe("TonkCore Creation", () => {
    it("should create a TonkCore with random peer ID", async () => {
      const tonk = await wasm.create_tonk();
      expect(tonk).to.not.be.undefined;

      const peerId = await tonk.getPeerId();
      expect(peerId).to.be.a("string");
      expect(peerId.length).to.be.greaterThan(0);
    });

    it("should create a TonkCore with specific peer ID", async () => {
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

    it("should create and check file existence", async () => {
      const path = "/test/hello.txt";
      const content = TestData.simpleText;

      await tonk.createFile(path, content);
      const exists = await tonk.exists(path);
      expect(exists).to.be.true;
    });

    it("should create directories", async () => {
      const path = "/documents";

      await tonk.createDirectory(path);
      const exists = await tonk.exists(path);
      expect(exists).to.be.true;
    });

    it("should handle nested directory creation", async () => {
      // Create parent directories first (createDirectory doesn't auto-create parents)
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
      await tonk.createFile("/docs/readme.md", "README content");
      await tonk.createFile("/docs/guide.md", "Guide content");

      const entries = await tonk.listDirectory("/docs");
      expect(entries).to.be.an("array");
      expect(entries.length).to.equal(2);

      const names = entries.map((entry) => entry.name);
      expect(names).to.include("readme.md");
      expect(names).to.include("guide.md");
    });

    it("should delete files", async () => {
      const path = "/temp/deleteme.txt";

      await tonk.createFile(path, "temporary content");
      expect(await tonk.exists(path)).to.be.true;

      const deleted = await tonk.deleteFile(path);
      expect(deleted).to.be.true;
      expect(await tonk.exists(path)).to.be.false;
    });

    it("should get file metadata", async () => {
      const path = "/data/info.json";
      const content = TestData.jsonConfig;

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

    it("should read file content", async () => {
      const path = "/readable.txt";
      const content = { message: "Hello, World!" };

      await tonk.createFile(path, content);
      const retrieved = await tonk.readFile(path);

      expect(retrieved).to.not.be.null;
      expect(retrieved.content).to.deep.equal(content);
    });

    it("should update file content", async () => {
      const path = "/updatable.txt";

      await tonk.createFile(path, { version: 1 });
      await tonk.setFile(path, { version: 2 });

      const retrieved = await tonk.readFile(path);
      expect(retrieved.content).to.deep.equal({ version: 2 });
    });

    it("should rename files", async () => {
      const oldPath = "/old-name.txt";
      const newPath = "/new-name.txt";

      await tonk.createFile(oldPath, "content");
      await tonk.rename(oldPath, newPath);

      expect(await tonk.exists(oldPath)).to.be.false;
      expect(await tonk.exists(newPath)).to.be.true;
    });
  });

  describe("Bundle Operations", () => {
    it("should export TonkCore to bytes and create bundle", async () => {
      // Create TonkCore with some content
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.txt", "Test content");
      await tonk.createDirectory("/folder");
      await tonk.createFile("/folder/nested.txt", "Nested content");

      // Export to bytes
      const bytes = await tonk.toBytes(null);
      expect(bytes).to.be.instanceOf(Uint8Array);
      expect(bytes.length).to.be.greaterThan(0);

      // Create bundle from bytes
      const bundle = wasm.create_bundle_from_bytes(bytes);
      expect(bundle).to.not.be.undefined;

      // List keys in bundle
      const keys = await bundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should retrieve data from bundle", async () => {
      // Create TonkCore with content
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/data.json", { key: "value" });

      // Export and create bundle
      const bytes = await tonk.toBytes(null);
      const bundle = wasm.create_bundle_from_bytes(bytes);

      // Get manifest
      const manifest = await bundle.getManifest();
      expect(manifest).to.be.an("object");

      // Get root ID
      const rootId = await bundle.getRootId();
      expect(rootId).to.be.a("string");
    });

    it("should load TonkCore from bundle bytes", async () => {
      // Create original TonkCore
      const tonk1 = await wasm.create_tonk();
      await tonk1.createFile("/persistent.txt", "Persisted data");

      // Export to bytes
      const bytes = await tonk1.toBytes(null);

      // Create new TonkCore from bytes
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      // Verify data persisted
      const exists = await tonk2.exists("/persistent.txt");
      expect(exists).to.be.true;

      const content = await tonk2.readFile("/persistent.txt");
      // Content is wrapped in { value: ... } for primitive types
      expect(content.content.value || content.content).to.equal("Persisted data");
    });

    it("should handle bundle serialization round-trip", async () => {
      // Create and populate TonkCore
      const tonk1 = await wasm.create_tonk();

      const testFiles = [
        { path: "/config.json", content: { theme: "dark" } },
        { path: "/readme.txt", content: "Hello World" },
        { path: "/data/nested.json", content: { nested: true } },
      ];

      for (const file of testFiles) {
        await tonk1.createFile(file.path, file.content);
      }

      // Export to bytes
      const bytes = await tonk1.toBytes(null);

      // Load into new TonkCore
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      // Verify all files exist and have correct content
      for (const file of testFiles) {
        const exists = await tonk2.exists(file.path);
        expect(exists).to.be.true;

        const retrieved = await tonk2.readFile(file.path);
        // Handle both wrapped primitives ({ value: ... }) and objects
        const actualContent = retrieved.content.value !== undefined 
          ? retrieved.content.value 
          : retrieved.content;
        expect(actualContent).to.deep.equal(file.content);
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
        await tonk.createFile("", "content");
        expect.fail("Expected error for empty path");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it("should handle duplicate file creation", async () => {
      const path = "/duplicate.txt";

      await tonk.createFile(path, "first content");

      try {
        await tonk.createFile(path, "second content");
        expect.fail("Expected error for duplicate file");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe("Performance", () => {
    it("should create multiple TonkCore instances efficiently", async () => {
      const timer = new PerfTimer("Multiple TonkCore creation");
      const instances = [];

      for (let i = 0; i < 10; i++) {
        instances.push(await wasm.create_tonk());
      }

      const duration = timer.stop();
      expect(instances).to.have.lengthOf(10);
      expect(duration).to.be.lessThan(2000); // Should complete within 2 seconds
    });

    it("should handle concurrent VFS operations", async () => {
      const tonk = await wasm.create_tonk();

      const timer = new PerfTimer("Concurrent VFS operations");
      const operations = [];

      // Create 50 files concurrently
      for (let i = 0; i < 50; i++) {
        operations.push(
          tonk.createFile(`/concurrent/file${i}.txt`, `Content ${i}`),
        );
      }

      await Promise.all(operations);
      const duration = timer.stop();

      // Verify all files exist
      for (let i = 0; i < 50; i++) {
        const exists = await tonk.exists(`/concurrent/file${i}.txt`);
        expect(exists).to.be.true;
      }

      expect(duration).to.be.lessThan(5000); // Should complete within 5 seconds
    });
  });
});
