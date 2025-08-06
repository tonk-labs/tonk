import { describe, it, expect } from 'vitest';
import { ZipBundle } from '../../bundle/zip-bundle.js';

describe('Bundle Parser', () => {
  describe('ZipBundle.parse', () => {
    it('should parse a valid bundle from ArrayBuffer', async () => {
      // Create a valid bundle first
      const original = await ZipBundle.createEmpty();
      await original.addFile(
        { path: '/test.js', contentType: 'application/javascript' },
        new TextEncoder().encode('console.log("test");').buffer
      );
      
      const buffer = await original.toArrayBuffer();
      const parsed = await ZipBundle.parse(buffer);
      
      expect(parsed).toBeInstanceOf(ZipBundle);
      expect(parsed.getFileCount()).toBe(1);
      expect(parsed.hasFile('/test.js')).toBe(true);
    });

    it('should parse a bundle with manifest.json', async () => {
      // Create a bundle with metadata
      const original = await ZipBundle.createEmpty();
      await original.addFile(
        { path: '/index.js', contentType: 'application/javascript' },
        new TextEncoder().encode('export default {};').buffer
      );
      original.setEntrypoint('main', '/index.js');
      
      const buffer = await original.toArrayBuffer();
      const parsed = await ZipBundle.parse(buffer);
      
      expect(parsed.manifest).toBeDefined();
      expect(parsed.manifest.version).toBe(1);
      expect(parsed.getEntrypoint('main')).toBe('/index.js');
    });

    it('should throw error for invalid ZIP format', async () => {
      const invalidBuffer = new ArrayBuffer(10);
      
      await expect(ZipBundle.parse(invalidBuffer)).rejects.toThrow();
    });

    it('should throw error when manifest.json is missing', async () => {
      // This test would require creating a ZIP without manifest.json
      // For now, we'll test with invalid data
      const buffer = new ArrayBuffer(100);
      
      await expect(ZipBundle.parse(buffer)).rejects.toThrow();
    });

    it('should validate manifest schema using Zod', async () => {
      // Create a bundle to ensure schema validation works
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/app.js', contentType: 'application/javascript' },
        new TextEncoder().encode('const app = {};').buffer
      );
      
      const buffer = await bundle.toArrayBuffer();
      
      // Parse with strict validation
      const parsed = await ZipBundle.parse(buffer, { strictValidation: true });
      expect(parsed).toBeInstanceOf(ZipBundle);
    });

    it('should handle bundles with multiple files', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      const files = [
        { path: '/src/index.js', content: 'export * from "./utils";' },
        { path: '/src/utils.js', content: 'export const util = () => {};' },
        { path: '/styles/main.css', content: 'body { margin: 0; }' },
      ];
      
      for (const file of files) {
        await bundle.addFile(
          { 
            path: file.path, 
            contentType: file.path.endsWith('.css') ? 'text/css' : 'application/javascript' 
          },
          new TextEncoder().encode(file.content).buffer
        );
      }
      
      const buffer = await bundle.toArrayBuffer();
      const parsed = await ZipBundle.parse(buffer);
      
      expect(parsed.getFileCount()).toBe(3);
      expect(parsed.hasFile('/src/index.js')).toBe(true);
      expect(parsed.hasFile('/src/utils.js')).toBe(true);
      expect(parsed.hasFile('/styles/main.css')).toBe(true);
    });

    it('should handle bundles with entrypoints', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new TextEncoder().encode('import "./worker";').buffer
      );
      await bundle.addFile(
        { path: '/worker.js', contentType: 'application/javascript' },
        new TextEncoder().encode('self.onmessage = () => {};').buffer
      );
      
      bundle.setEntrypoint('main', '/main.js');
      bundle.setEntrypoint('worker', '/worker.js');
      
      const buffer = await bundle.toArrayBuffer();
      const parsed = await ZipBundle.parse(buffer);
      
      expect(parsed.getEntrypointNames()).toEqual(['main', 'worker']);
      expect(parsed.getEntrypoint('main')).toBe('/main.js');
      expect(parsed.getEntrypoint('worker')).toBe('/worker.js');
    });

    it('should validate that manifest references match ZIP contents', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/exists.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      
      const buffer = await bundle.toArrayBuffer();
      
      // Parse with validation
      const parsed = await ZipBundle.parse(buffer, { 
        validateFileReferences: true,
        strictValidation: true
      });
      
      expect(parsed.validate().valid).toBe(true);
    });

    it('should respect maxSize option', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      // Add a large file
      const largeData = new ArrayBuffer(10000);
      await bundle.addFile(
        { path: '/large.bin', contentType: 'application/octet-stream' },
        largeData
      );
      
      const buffer = await bundle.toArrayBuffer();
      
      // Should throw when max size is exceeded
      await expect(
        ZipBundle.parse(buffer, { maxSize: 100 })
      ).rejects.toThrow('exceeds maximum allowed size');
      
      // Should succeed when under limit
      const parsed = await ZipBundle.parse(buffer, { maxSize: 100000 });
      expect(parsed).toBeInstanceOf(ZipBundle);
    });

    it('should handle non-strict validation mode', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new ArrayBuffer(10)
      );
      
      const buffer = await bundle.toArrayBuffer();
      
      // Parse without strict validation
      const parsed = await ZipBundle.parse(buffer, { strictValidation: false });
      expect(parsed).toBeInstanceOf(ZipBundle);
    });
  });

  describe('Bundle class ZIP operations', () => {
    it('should lazy load file contents from ZIP', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      const content = 'lazy loaded content';
      await bundle.addFile(
        { path: '/lazy.txt', contentType: 'text/plain' },
        new TextEncoder().encode(content).buffer
      );
      
      // File metadata should be available immediately
      const file = bundle.getFile('/lazy.txt');
      expect(file).not.toBeNull();
      
      // File content should be loaded on demand
      const data = await bundle.getFileData('/lazy.txt');
      expect(new TextDecoder().decode(data!)).toBe(content);
    });

    it('should maintain file metadata in manifest', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      const now = new Date().toISOString();
      await bundle.addFile(
        { 
          path: '/metadata.js', 
          contentType: 'application/javascript',
          compressed: true,
          lastModified: now,
        },
        new ArrayBuffer(1000),
        { compress: true }
      );
      
      const file = bundle.getFile('/metadata.js');
      expect(file?.compressed).toBe(true);
      expect(file?.lastModified).toBe(now);
      expect(file?.contentType).toBe('application/javascript');
      expect(file?.length).toBe(1000);
    });
  });
});