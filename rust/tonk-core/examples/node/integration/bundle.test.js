/**
 * Bundle operations integration tests
 *
 * Note: The WasmBundle API is read-only - bundles are created by exporting
 * a TonkCore instance to bytes via toBytes(), then loading with create_bundle_from_bytes().
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

  /**
   * Helper to create a bundle with test data
   * Since WasmBundle is read-only, we create a TonkCore, populate it, then export to bytes
   */
  async function createPopulatedBundle(files) {
    const tonk = await wasm.create_tonk();
    for (const file of files) {
      await tonk.createFile(file.path, file.content);
    }
    const bytes = await tonk.toBytes();
    return { bundle: wasm.create_bundle_from_bytes(bytes), tonk, bytes };
  }

  describe("Bundle Creation and Basic Operations", () => {
    it("should create bundle from tonk bytes", async () => {
      const testFiles = [
        { path: "/config.json", content: { theme: "dark", language: "en" } },
        { path: "/readme.json", content: { text: TestData.simpleText } },
        {
          path: "/data/info.json",
          content: { binary: Array.from(TestData.binaryData) },
        },
      ];

      const { bundle } = await createPopulatedBundle(testFiles);

      // Verify bundle was created
      expect(bundle).to.not.be.undefined;

      // Verify keys exist
      const keys = await bundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should handle hierarchical paths", async () => {
      const hierarchicalFiles = [
        { path: "/root.json", content: { level: 0 } },
        { path: "/dir1/file1.json", content: { level: 1 } },
        { path: "/dir1/file2.json", content: { level: 1 } },
        { path: "/dir1/subdir/file3.json", content: { level: 2 } },
        { path: "/dir2/another.json", content: { level: 1 } },
      ];

      const { bundle } = await createPopulatedBundle(hierarchicalFiles);

      const keys = await bundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);
    });

    it("should preserve data integrity through export/import cycle", async () => {
      const testContent = { test: true, nested: { value: 42 } };

      // Create tonk and add file
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/integrity-test.json", testContent);

      // Export to bytes
      const bytes = await tonk.toBytes();
      expect(bytes).to.be.instanceOf(Uint8Array);

      // Create new tonk from bytes
      const restoredTonk = await wasm.create_tonk_from_bytes(bytes);

      // Verify data is preserved
      const exists = await restoredTonk.exists("/integrity-test.json");
      expect(exists).to.be.true;

      // readFile returns full doc with metadata, content is in .content
      const doc = await restoredTonk.readFile("/integrity-test.json");
      expect(doc.content).to.deep.equal(testContent);
    });

    it("should handle overwrites correctly in tonk before export", async () => {
      const tonk = await wasm.create_tonk();
      const path = "/overwrite-test.json";

      // Initial content
      await tonk.createFile(path, { version: 1 });
      let doc = await tonk.readFile(path);
      expect(doc.content.version).to.equal(1);

      // Overwrite using setFile
      await tonk.setFile(path, { version: 2, updated: true });
      doc = await tonk.readFile(path);
      expect(doc.content.version).to.equal(2);
      expect(doc.content.updated).to.be.true;

      // Export and verify
      const bytes = await tonk.toBytes();
      const restoredTonk = await wasm.create_tonk_from_bytes(bytes);
      const finalDoc = await restoredTonk.readFile(path);
      expect(finalDoc.content.version).to.equal(2);
    });
  });

  describe("Bundle Serialization", () => {
    it("should serialize and deserialize bundle data", async () => {
      // Create and populate a tonk
      const tonk = await wasm.create_tonk();

      const testData = [
        { path: "/file1.json", content: { message: "Hello World" } },
        { path: "/dir/file2.json", content: { test: true } },
        {
          path: "/binary.json",
          content: { data: Array.from(TestData.binaryData) },
        },
      ];

      for (const item of testData) {
        await tonk.createFile(item.path, item.content);
      }

      // Serialize to bytes
      const serialized = await tonk.toBytes();
      expect(serialized).to.be.instanceOf(Uint8Array);
      expect(serialized.length).to.be.greaterThan(0);

      // Create bundle from serialized data
      const bundle = wasm.create_bundle_from_bytes(serialized);
      expect(bundle).to.not.be.undefined;

      // Also verify we can create a new tonk from the bytes
      const deserializedTonk = await wasm.create_tonk_from_bytes(serialized);

      // Verify all data is preserved
      for (const item of testData) {
        const exists = await deserializedTonk.exists(item.path);
        expect(exists).to.be.true;

        const doc = await deserializedTonk.readFile(item.path);
        expect(doc.content).to.deep.equal(item.content);
      }
    });

    it("should handle empty tonk serialization", async () => {
      const tonk = await wasm.create_tonk();

      const serialized = await tonk.toBytes();
      expect(serialized).to.be.instanceOf(Uint8Array);

      // Should be able to load from empty bytes
      const bundle = wasm.create_bundle_from_bytes(serialized);
      const keys = await bundle.listKeys();
      // Empty tonk should still have some internal keys (manifest, root doc)
      expect(keys).to.be.an("array");
    });
  });

  describe("Bundle Performance", () => {
    it("should handle large numbers of files efficiently", async function () {
      this.timeout(60000);

      const tonk = await wasm.create_tonk();
      const fileCount = 500; // Reduced from 1000 for reasonable test time

      const timer = new PerfTimer(`Storing ${fileCount} files`);

      // Store many small files
      for (let i = 0; i < fileCount; i++) {
        const path = `/files/file_${i.toString().padStart(4, "0")}.json`;
        await tonk.createFile(path, {
          index: i,
          content: `Content of file ${i}`,
        });
      }

      const storeTime = timer.stop();

      // Export to bytes
      const exportTimer = new PerfTimer("Exporting to bytes");
      const bytes = await tonk.toBytes();
      const exportTime = exportTimer.stop();

      // Create bundle and list keys
      const bundle = wasm.create_bundle_from_bytes(bytes);
      const keys = await bundle.listKeys();
      expect(keys.length).to.be.greaterThan(0);

      console.log(
        `    Store rate: ${((fileCount / storeTime) * 1000).toFixed(0)} files/sec`,
      );
      console.log(`    Export time: ${exportTime.toFixed(2)}ms`);

      expect(storeTime).to.be.lessThan(30000);
      expect(exportTime).to.be.lessThan(5000);
    });

    it("should handle large files efficiently", async function () {
      this.timeout(10000);

      const tonk = await wasm.create_tonk();
      // 100KB of text content
      const largeContent = { text: "x".repeat(100 * 1024) };

      const timer = new PerfTimer("Large file operations");

      await tonk.createFile("/large-file.json", largeContent);
      const bytes = await tonk.toBytes();

      // Restore and verify
      const restoredTonk = await wasm.create_tonk_from_bytes(bytes);
      const doc = await restoredTonk.readFile("/large-file.json");

      const duration = timer.stop();

      expect(doc.content.text.length).to.equal(largeContent.text.length);
      expect(duration).to.be.lessThan(5000);
    });
  });

  describe("Bundle Error Handling", () => {
    it("should handle non-existent files gracefully", async () => {
      const tonk = await wasm.create_tonk();

      const content = await tonk.readFile("/non-existent-file.json");
      expect(content).to.be.null;
    });

    it("should handle deletion gracefully", async () => {
      const tonk = await wasm.create_tonk();

      // Delete non-existent file should return false
      const deleted = await tonk.deleteFile("/non-existent.json");
      expect(deleted).to.be.false;
    });

    it("should handle invalid serialized data", async () => {
      const invalidData = new Uint8Array([1, 2, 3, 4, 5]); // Invalid bundle data

      try {
        wasm.create_bundle_from_bytes(invalidData);
        expect.fail("Expected error for invalid bundle data");
      } catch (error) {
        expect(error).to.not.be.undefined;
      }
    });
  });

  describe("Bundle Integration with File System", () => {
    it("should save bundle to file and load it back", async () => {
      // Create and populate tonk
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.json", { content: "test content" });

      // Serialize to bytes
      const bundleBytes = await tonk.toBytes();

      // Save to temporary file
      const tempFile = createTempFile("", ".tonk");
      fs.writeFileSync(tempFile.name, bundleBytes);

      // Read from file and create bundle
      const fileData = fs.readFileSync(tempFile.name);
      const loadedBundle = wasm.create_bundle_from_bytes(
        new Uint8Array(fileData),
      );

      // Verify bundle loads
      const keys = await loadedBundle.listKeys();
      expect(keys).to.be.an("array");
      expect(keys.length).to.be.greaterThan(0);

      // Also verify we can create a tonk from the file data
      const loadedTonk = await wasm.create_tonk_from_bytes(
        new Uint8Array(fileData),
      );
      const exists = await loadedTonk.exists("/test.json");
      expect(exists).to.be.true;

      const doc = await loadedTonk.readFile("/test.json");
      expect(doc.content).to.deep.equal({ content: "test content" });

      // Cleanup
      tempFile.removeCallback();
    });
  });

  describe("Bundle Manifest", () => {
    it("should access bundle manifest", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.json", { data: true });

      const bytes = await tonk.toBytes();
      const bundle = wasm.create_bundle_from_bytes(bytes);

      const manifest = await bundle.getManifest();
      expect(manifest).to.be.an("object");
    });

    it("should get bundle root ID", async () => {
      const tonk = await wasm.create_tonk();
      await tonk.createFile("/test.json", { data: true });

      const bytes = await tonk.toBytes();
      const bundle = wasm.create_bundle_from_bytes(bytes);

      const rootId = await bundle.getRootId();
      expect(rootId).to.be.a("string");
      expect(rootId.length).to.be.greaterThan(0);
    });
  });

  describe("Bundle Prefix Operations", () => {
    it("should retrieve entries by prefix", async () => {
      const tonk = await wasm.create_tonk();

      // Create files with common prefix
      await tonk.createFile("/prefix/file1.json", { name: "file1" });
      await tonk.createFile("/prefix/file2.json", { name: "file2" });
      await tonk.createFile("/other/file3.json", { name: "file3" });

      const bytes = await tonk.toBytes();
      const bundle = wasm.create_bundle_from_bytes(bytes);

      // Get entries with prefix
      const prefixEntries = await bundle.getPrefix("docs/prefix/");
      expect(prefixEntries).to.be.an("array");
    });
  });
});
