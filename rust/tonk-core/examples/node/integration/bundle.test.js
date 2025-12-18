/**
 * Bundle operations integration tests
 * 
 * Note: Bundles in the WASM API are created by exporting TonkCore state via toBytes().
 * Bundles are read-only - you cannot directly write to them.
 */

const { expect } = require("chai");
const fs = require("fs");
const {
  initWasm,
  createTempFile,
  TestData,
  assertUint8ArraysEqual,
  PerfTimer,
} = require("../../shared/test-utils");

describe("Bundle Integration Tests", () => {
  let wasm;

  before(async function () {
    this.timeout(10000);
    wasm = await initWasm();
  });

  describe("Bundle Creation via TonkCore Export", () => {
    it("should create bundle from TonkCore with multiple files", async () => {
      const tonk = await wasm.create_tonk();

      // Add test files
      const testFiles = [
        { path: "/config.json", content: JSON.parse(TestData.jsonConfig) },
        { path: "/readme.txt", content: TestData.simpleText },
        { path: "/data/info.json", content: { type: "info", version: 1 } },
      ];

      for (const file of testFiles) {
        await tonk.createFile(file.path, file.content);
      }

      // Export to bundle bytes
      const bytes = await tonk.toBytes(null);
      expect(bytes).to.be.instanceOf(Uint8Array);
      expect(bytes.length).to.be.greaterThan(0);

      // Create bundle from bytes
      const bundle = wasm.create_bundle_from_bytes(bytes);
      expect(bundle).to.not.be.undefined;

      // Verify bundle has keys
      const keys = await bundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should handle hierarchical paths in bundle", async () => {
      const tonk = await wasm.create_tonk();

      const hierarchicalFiles = [
        "/root.txt",
        "/dir1/file1.txt",
        "/dir1/file2.txt",
        "/dir1/subdir/file3.txt",
        "/dir2/another.txt",
      ];

      for (const [index, path] of hierarchicalFiles.entries()) {
        await tonk.createFile(path, `Content ${index}: ${path}`);
      }

      // Export and verify structure preserved
      const bytes = await tonk.toBytes(null);
      const bundle = wasm.create_bundle_from_bytes(bytes);

      const keys = await bundle.listKeys();
      expect(keys.length).to.be.greaterThan(0);

      // Load back into TonkCore and verify all files
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      for (const path of hierarchicalFiles) {
        const exists = await tonk2.exists(path);
        expect(exists).to.be.true;
      }
    });

    it("should preserve data integrity through bundle round-trip", async () => {
      const tonk1 = await wasm.create_tonk();

      // Test with various data types
      const testCases = [
        { path: "/text.txt", content: TestData.simpleText },
        { path: "/json.json", content: { theme: "dark", language: "en" } },
        { path: "/nested.json", content: { a: { b: { c: "deep" } } } },
        { path: "/array.json", content: [1, 2, 3, "four", { five: 5 }] },
      ];

      // Store all data
      for (const testCase of testCases) {
        await tonk1.createFile(testCase.path, testCase.content);
      }

      // Export and reload
      const bytes = await tonk1.toBytes(null);
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);

      // Retrieve and verify
      for (const testCase of testCases) {
        const retrieved = await tonk2.readFile(testCase.path);
        // Handle both wrapped primitives ({ value: ... }) and objects
        const actualContent = retrieved.content.value !== undefined
          ? retrieved.content.value
          : retrieved.content;
        expect(actualContent).to.deep.equal(testCase.content);
      }
    });
  });

  describe("Bundle Reading Operations", () => {
    let bundle, bytes;

    beforeEach(async () => {
      // Create a TonkCore with test data
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.txt", "Test content");
      await tonk.createFile("/data.json", { key: "value" });
      await tonk.createDirectory("/folder");
      await tonk.createFile("/folder/nested.txt", "Nested");

      bytes = await tonk.toBytes(null);
      bundle = wasm.create_bundle_from_bytes(bytes);
    });

    it("should list all keys in bundle", async () => {
      const keys = await bundle.listKeys();

      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should get manifest from bundle", async () => {
      const manifest = await bundle.getManifest();

      expect(manifest).to.be.an("object");
    });

    it("should get root ID from bundle", async () => {
      const rootId = await bundle.getRootId();

      expect(rootId).to.be.a("string");
      expect(rootId.length).to.be.greaterThan(0);
    });

    it("should get raw data by key", async () => {
      const keys = await bundle.listKeys();

      // Try to get each key
      for (const key of keys) {
        const data = await bundle.get(key);
        // Data should be either Uint8Array or null
        if (data !== null) {
          expect(data).to.be.instanceOf(Uint8Array);
        }
      }
    });

    it("should get entries by prefix", async () => {
      const entries = await bundle.getPrefix("");

      expect(entries).to.be.an("array");
      // Each entry should have key and value
      for (const entry of entries) {
        expect(entry).to.have.property("key");
        expect(entry).to.have.property("value");
      }
    });

    it("should serialize bundle to bytes", async () => {
      const serialized = await bundle.toBytes();

      expect(serialized).to.be.instanceOf(Uint8Array);
      expect(serialized.length).to.be.greaterThan(0);

      // Should be able to create another bundle from these bytes
      const bundle2 = wasm.create_bundle_from_bytes(serialized);
      const keys2 = await bundle2.listKeys();

      expect(keys2.length).to.be.greaterThan(0);
    });
  });

  describe("Bundle Performance", () => {
    it("should handle large numbers of files efficiently", async function () {
      this.timeout(20000);

      const tonk = await wasm.create_tonk();
      const fileCount = 500;

      const timer = new PerfTimer(`Creating ${fileCount} files`);

      // Create many files
      for (let i = 0; i < fileCount; i++) {
        const path = `/files/file_${i.toString().padStart(4, "0")}.txt`;
        await tonk.createFile(path, `Content of file ${i}`);
      }

      const createTime = timer.stop();

      // Export to bundle
      const exportTimer = new PerfTimer("Exporting to bundle");
      const bytes = await tonk.toBytes(null);
      const exportTime = exportTimer.stop();

      // Load from bundle
      const loadTimer = new PerfTimer("Loading from bundle");
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);
      const loadTime = loadTimer.stop();

      // Verify file count
      const entries = await tonk2.listDirectory("/files");
      expect(entries).to.have.lengthOf(fileCount);

      console.log(`    Create rate: ${((fileCount / createTime) * 1000).toFixed(0)} files/sec`);
      console.log(`    Export time: ${exportTime.toFixed(2)}ms`);
      console.log(`    Load time: ${loadTime.toFixed(2)}ms`);

      expect(createTime).to.be.lessThan(20000);
      expect(exportTime).to.be.lessThan(5000);
      expect(loadTime).to.be.lessThan(5000);
    });

    it("should handle large file content efficiently", async function () {
      this.timeout(10000);

      const tonk = await wasm.create_tonk();
      const largeContent = "x".repeat(100 * 1024); // 100KB string

      const timer = new PerfTimer("Large content operations");

      await tonk.createFile("/large-file.txt", largeContent);
      const bytes = await tonk.toBytes(null);
      const tonk2 = await wasm.create_tonk_from_bytes(bytes);
      const retrieved = await tonk2.readFile("/large-file.txt");

      const duration = timer.stop();

      // Handle wrapped primitive
      const actualContent = retrieved.content.value !== undefined
        ? retrieved.content.value
        : retrieved.content;
      expect(actualContent).to.equal(largeContent);
      expect(duration).to.be.lessThan(3000);
    });
  });

  describe("Bundle Error Handling", () => {
    it("should handle invalid bundle data gracefully", async () => {
      const invalidData = new Uint8Array([1, 2, 3, 4, 5]); // Invalid bundle data

      try {
        wasm.create_bundle_from_bytes(invalidData);
        expect.fail("Expected error for invalid bundle data");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });

    it("should handle empty Uint8Array", async () => {
      const emptyData = new Uint8Array(0);

      try {
        wasm.create_bundle_from_bytes(emptyData);
        expect.fail("Expected error for empty data");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe("Bundle Integration with File System", () => {
    it("should save bundle to file and load it back", async () => {
      // Create TonkCore with content
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.txt", "test content");

      // Export to bytes
      const bundleBytes = await tonk.toBytes(null);

      // Save to temporary file
      const tempFile = createTempFile("", ".tonk");
      fs.writeFileSync(tempFile.name, bundleBytes);

      // Read from file
      const fileData = fs.readFileSync(tempFile.name);
      const loadedBytes = new Uint8Array(fileData);

      // Create TonkCore from loaded bytes
      const tonk2 = await wasm.create_tonk_from_bytes(loadedBytes);

      // Verify data
      const exists = await tonk2.exists("/test.txt");
      expect(exists).to.be.true;

      const content = await tonk2.readFile("/test.txt");
      // Handle wrapped primitive
      const actualContent = content.content.value !== undefined
        ? content.content.value
        : content.content;
      expect(actualContent).to.equal("test content");

      // Cleanup
      tempFile.removeCallback();
    });

    it("should handle multiple save/load cycles", async () => {
      let tonk = await wasm.create_tonk();
      await tonk.createFile("/cycle-test.txt", "initial");

      // Perform multiple save/load cycles
      for (let cycle = 1; cycle <= 3; cycle++) {
        // Update content
        await tonk.setFile("/cycle-test.txt", `cycle ${cycle}`);

        // Save and reload
        const bytes = await tonk.toBytes(null);
        tonk = await wasm.create_tonk_from_bytes(bytes);

        // Verify
        const content = await tonk.readFile("/cycle-test.txt");
        // Handle wrapped primitive
        const actualContent = content.content.value !== undefined
          ? content.content.value
          : content.content;
        expect(actualContent).to.equal(`cycle ${cycle}`);
      }
    });
  });

  describe("TonkCore from Bundle with Storage Config", () => {
    it("should create TonkCore from bundle with in-memory storage", async () => {
      // Create original TonkCore
      const tonk1 = await wasm.create_tonk();
      await tonk1.createFile("/storage-test.txt", "Storage test content");

      // Export to bytes
      const bytes = await tonk1.toBytes(null);

      // Create bundle and then TonkCore with storage config
      const bundle = wasm.create_bundle_from_bytes(bytes);
      const tonk2 = await wasm.create_tonk_from_bundle(bundle);

      // Verify data
      const exists = await tonk2.exists("/storage-test.txt");
      expect(exists).to.be.true;
    });

    it("should create TonkCore from bytes with storage config", async () => {
      // Create original
      const tonk1 = await wasm.create_tonk();
      await tonk1.createFile("/bytes-storage.txt", "Bytes storage test");

      // Export
      const bytes = await tonk1.toBytes(null);

      // Load with storage config (in-memory)
      const tonk2 = await wasm.create_tonk_from_bytes_with_storage(bytes, false, null);

      // Verify
      const content = await tonk2.readFile("/bytes-storage.txt");
      // Handle wrapped primitive
      const actualContent = content.content.value !== undefined
        ? content.content.value
        : content.content;
      expect(actualContent).to.equal("Bytes storage test");
    });
  });
});
