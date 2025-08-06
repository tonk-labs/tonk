import { describe, it, expect } from 'vitest';
import { ZipBundle } from '../../bundle/zip-bundle.js';

describe('Bundle Integration Tests', () => {
  describe('Parsing and Serialization Roundtrips', () => {
    it('should parse and serialize a simple bundle', async () => {
      // Create a simple bundle
      const files = new Map<string, ArrayBuffer>([
        ['index.js', new TextEncoder().encode('console.log("Hello World");').buffer],
        ['style.css', new TextEncoder().encode('body { color: red; }').buffer],
      ]);

      const bundle = await ZipBundle.fromFiles(files);
      expect(bundle).toBeInstanceOf(ZipBundle);
      expect(bundle.getFileCount()).toBe(2);
    });

    it('should maintain file integrity through roundtrip', async () => {
      const originalData = new TextEncoder().encode('const x = 42;').buffer;
      const files = new Map<string, ArrayBuffer>([
        ['src/main.js', originalData],
      ]);

      const bundle = await ZipBundle.fromFiles(files);
      const serialized = await bundle.toArrayBuffer();
      const parsed = await ZipBundle.parse(serialized);
      
      const retrievedData = await parsed.getFileData('/src/main.js');
      const retrievedText = new TextDecoder().decode(retrievedData!);
      expect(retrievedText).toBe('const x = 42;');
    });

    it('should preserve entrypoints through roundtrip', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['main.js', new TextEncoder().encode('// main').buffer],
        ['worker.js', new TextEncoder().encode('// worker').buffer],
      ]);

      const bundle = await ZipBundle.fromFiles(files);
      bundle.setEntrypoint('main', '/main.js');
      bundle.setEntrypoint('worker', '/worker.js');
      
      const serialized = await bundle.toArrayBuffer();
      const parsed = await ZipBundle.parse(serialized);
      
      expect(parsed.getEntrypoint('main')).toBe('/main.js');
      expect(parsed.getEntrypoint('worker')).toBe('/worker.js');
    });

    it('should handle UTF-8 filenames correctly', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['文档.txt', new TextEncoder().encode('UTF-8 content').buffer],
        ['файл.js', new TextEncoder().encode('// Russian filename').buffer],
      ]);

      const bundle = await ZipBundle.fromFiles(files);
      expect(bundle.hasFile('/文档.txt')).toBe(true);
      expect(bundle.hasFile('/файл.js')).toBe(true);
      
      const data = await bundle.getFileData('/文档.txt');
      expect(new TextDecoder().decode(data!)).toBe('UTF-8 content');
    });
  });

  describe('Bundle Modification and Serialization', () => {
    it('should add a file and serialize correctly', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/new.js', contentType: 'application/javascript' },
        new TextEncoder().encode('export const x = 1;').buffer
      );
      
      const serialized = await bundle.toArrayBuffer();
      const parsed = await ZipBundle.parse(serialized);
      
      expect(parsed.hasFile('/new.js')).toBe(true);
      const data = await parsed.getFileData('/new.js');
      expect(new TextDecoder().decode(data!)).toBe('export const x = 1;');
    });

    it('should remove a file and update manifest', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile({ path: '/a.js', contentType: 'text/javascript' }, new ArrayBuffer(10));
      await bundle.addFile({ path: '/b.js', contentType: 'text/javascript' }, new ArrayBuffer(20));
      
      expect(bundle.getFileCount()).toBe(2);
      
      await bundle.removeFile('/a.js');
      
      expect(bundle.getFileCount()).toBe(1);
      expect(bundle.hasFile('/a.js')).toBe(false);
      expect(bundle.hasFile('/b.js')).toBe(true);
    });

    it('should update file content and maintain structure', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/config.json', contentType: 'application/json' },
        new TextEncoder().encode('{"version": 1}').buffer
      );
      
      await bundle.updateFile(
        '/config.json',
        new TextEncoder().encode('{"version": 2}').buffer
      );
      
      const data = await bundle.getFileData('/config.json');
      expect(new TextDecoder().decode(data!)).toBe('{"version": 2}');
    });

    it('should handle entrypoint modifications', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile({ path: '/app.js', contentType: 'application/javascript' }, new ArrayBuffer(10));
      await bundle.addFile({ path: '/lib.js', contentType: 'application/javascript' }, new ArrayBuffer(10));
      
      bundle.setEntrypoint('app', '/app.js');
      expect(bundle.getEntrypoint('app')).toBe('/app.js');
      
      bundle.setEntrypoint('app', '/lib.js');
      expect(bundle.getEntrypoint('app')).toBe('/lib.js');
      
      bundle.removeEntrypoint('app');
      expect(bundle.hasEntrypoint('app')).toBe(false);
    });
  });

  describe('Large Bundle Operations', () => {
    it('should handle bundles with many small files', async () => {
      const files = new Map<string, ArrayBuffer>();
      for (let i = 0; i < 100; i++) {
        files.set(`file${i}.txt`, new TextEncoder().encode(`Content ${i}`).buffer);
      }

      const bundle = await ZipBundle.fromFiles(files);
      expect(bundle.getFileCount()).toBe(100);
      
      // Verify random file
      const data = await bundle.getFileData('/file42.txt');
      expect(new TextDecoder().decode(data!)).toBe('Content 42');
    });

    it('should handle bundles with large files', async () => {
      // Create a 1MB file
      const largeData = new ArrayBuffer(1024 * 1024);
      const view = new Uint8Array(largeData);
      for (let i = 0; i < view.length; i++) {
        view[i] = i % 256;
      }
      
      const files = new Map<string, ArrayBuffer>([
        ['large.bin', largeData],
      ]);

      const bundle = await ZipBundle.fromFiles(files);
      const retrievedData = await bundle.getFileData('/large.bin');
      
      expect(retrievedData!.byteLength).toBe(1024 * 1024);
    });
  });

  describe('Bundle Cloning', () => {
    it('should create an independent copy', async () => {
      const original = await ZipBundle.createEmpty();
      
      await original.addFile(
        { path: '/original.js', contentType: 'application/javascript' },
        new TextEncoder().encode('original content').buffer
      );
      original.setEntrypoint('main', '/original.js');
      
      const clone = await original.clone();
      
      // Modify original
      await original.addFile(
        { path: '/new.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      
      // Clone should not have the new file
      expect(clone.hasFile('/new.js')).toBe(false);
      expect(clone.hasFile('/original.js')).toBe(true);
    });

    it('should preserve all metadata in clone', async () => {
      const original = await ZipBundle.createEmpty();
      
      const timestamp = new Date().toISOString();
      await original.addFile(
        {
          path: '/meta.js',
          contentType: 'application/javascript',
          compressed: true,
          lastModified: timestamp,
          uncompressedSize: 1000,
        },
        new ArrayBuffer(500),
        { compress: true }
      );
      
      const clone = await original.clone();
      const file = clone.getFile('/meta.js');
      
      expect(file?.compressed).toBe(true);
      expect(file?.lastModified).toBe(timestamp);
      expect(file?.uncompressedSize).toBe(1000);
    });
  });

  describe('Bundle Merging', () => {
    it('should merge two bundles without conflicts', async () => {
      const bundle1 = await ZipBundle.createEmpty();
      await bundle1.addFile({ path: '/a.js', contentType: 'text/javascript' }, new ArrayBuffer(10));
      
      const bundle2 = await ZipBundle.createEmpty();
      await bundle2.addFile({ path: '/b.js', contentType: 'text/javascript' }, new ArrayBuffer(20));
      
      // Manual merge for now (merge method not implemented)
      const merged = await ZipBundle.createEmpty();
      
      for (const file of bundle1.listFiles()) {
        const data = await bundle1.getFileData(file.path);
        await merged.addFile(file, data!);
      }
      
      for (const file of bundle2.listFiles()) {
        const data = await bundle2.getFileData(file.path);
        await merged.addFile(file, data!);
      }
      
      expect(merged.getFileCount()).toBe(2);
      expect(merged.hasFile('/a.js')).toBe(true);
      expect(merged.hasFile('/b.js')).toBe(true);
    });

    it('should handle file conflicts during merge', async () => {
      const bundle1 = await ZipBundle.createEmpty();
      await bundle1.addFile(
        { path: '/shared.js', contentType: 'text/javascript' },
        new TextEncoder().encode('bundle1 version').buffer
      );
      
      const bundle2 = await ZipBundle.createEmpty();
      await bundle2.addFile(
        { path: '/shared.js', contentType: 'text/javascript' },
        new TextEncoder().encode('bundle2 version').buffer
      );
      
      // Merge with replace strategy
      const merged = await ZipBundle.createEmpty();
      
      for (const file of bundle1.listFiles()) {
        const data = await bundle1.getFileData(file.path);
        await merged.addFile(file, data!);
      }
      
      for (const file of bundle2.listFiles()) {
        const data = await bundle2.getFileData(file.path);
        await merged.addFile(file, data!, { replace: true });
      }
      
      const data = await merged.getFileData('/shared.js');
      expect(new TextDecoder().decode(data!)).toBe('bundle2 version');
    });

    it('should merge entrypoints correctly', async () => {
      const bundle1 = await ZipBundle.createEmpty();
      await bundle1.addFile({ path: '/a.js', contentType: 'text/javascript' }, new ArrayBuffer(10));
      bundle1.setEntrypoint('a', '/a.js');
      
      const bundle2 = await ZipBundle.createEmpty();
      await bundle2.addFile({ path: '/b.js', contentType: 'text/javascript' }, new ArrayBuffer(20));
      bundle2.setEntrypoint('b', '/b.js');
      
      // Manual merge
      const merged = await ZipBundle.createEmpty();
      
      // Merge files
      for (const file of [...bundle1.listFiles(), ...bundle2.listFiles()]) {
        const sourceBundle = bundle1.hasFile(file.path) ? bundle1 : bundle2;
        const data = await sourceBundle.getFileData(file.path);
        await merged.addFile(file, data!);
      }
      
      // Merge entrypoints
      for (const [name, path] of Object.entries(bundle1.listEntrypoints())) {
        merged.setEntrypoint(name, path);
      }
      for (const [name, path] of Object.entries(bundle2.listEntrypoints())) {
        merged.setEntrypoint(name, path);
      }
      
      expect(merged.getEntrypointNames().sort()).toEqual(['a', 'b']);
    });
  });
});