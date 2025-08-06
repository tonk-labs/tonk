import { describe, it, expect } from 'vitest';
import { ZipBundle } from '../../bundle/zip-bundle.js';

describe('ZipBundle', () => {
  describe('File Access Methods', () => {
    it('should get a file by path', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['index.js', new TextEncoder().encode('console.log("hello");').buffer],
      ]);
      const bundle = await ZipBundle.fromFiles(files);
      
      const file = bundle.getFile('/index.js');
      expect(file).not.toBeNull();
      expect(file?.path).toBe('/index.js');
      expect(file?.contentType).toBe('application/octet-stream');
    });

    it('should get file data by path', async () => {
      const content = 'console.log("hello");';
      const files = new Map<string, ArrayBuffer>([
        ['index.js', new TextEncoder().encode(content).buffer],
      ]);
      const bundle = await ZipBundle.fromFiles(files);
      
      const data = await bundle.getFileData('/index.js');
      expect(data).not.toBeNull();
      const text = new TextDecoder().decode(data!);
      expect(text).toBe(content);
    });

    it('should check if a file exists', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['index.js', new TextEncoder().encode('test').buffer],
      ]);
      const bundle = await ZipBundle.fromFiles(files);
      
      expect(bundle.hasFile('/index.js')).toBe(true);
      expect(bundle.hasFile('/missing.js')).toBe(false);
    });

    it('should list all files', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['index.js', new TextEncoder().encode('test1').buffer],
        ['style.css', new TextEncoder().encode('test2').buffer],
      ]);
      const bundle = await ZipBundle.fromFiles(files);
      
      const fileList = bundle.listFiles();
      expect(fileList).toHaveLength(2);
      expect(fileList.map(f => f.path).sort()).toEqual(['/index.js', '/style.css']);
    });

    it('should get file count', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['a.js', new ArrayBuffer(10)],
        ['b.js', new ArrayBuffer(20)],
        ['c.js', new ArrayBuffer(30)],
      ]);
      const bundle = await ZipBundle.fromFiles(files);
      
      expect(bundle.getFileCount()).toBe(3);
    });
  });

  describe('Entrypoint Methods', () => {
    it('should get an entrypoint by name', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      bundle.setEntrypoint('main', '/main.js');
      
      expect(bundle.getEntrypoint('main')).toBe('/main.js');
      expect(bundle.getEntrypoint('missing')).toBeNull();
    });

    it('should check if an entrypoint exists', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      bundle.setEntrypoint('main', '/main.js');
      
      expect(bundle.hasEntrypoint('main')).toBe(true);
      expect(bundle.hasEntrypoint('missing')).toBe(false);
    });

    it('should list all entrypoints', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      await bundle.addFile(
        { path: '/worker.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      bundle.setEntrypoint('main', '/main.js');
      bundle.setEntrypoint('worker', '/worker.js');
      
      const entrypoints = bundle.listEntrypoints();
      expect(entrypoints).toEqual({
        main: '/main.js',
        worker: '/worker.js',
      });
    });

    it('should get entrypoint names', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      bundle.setEntrypoint('main', '/main.js');
      bundle.setEntrypoint('secondary', '/main.js');
      
      const names = bundle.getEntrypointNames();
      expect(names).toHaveLength(2);
      expect(names.sort()).toEqual(['main', 'secondary']);
    });
  });

  describe('File Modification Methods', () => {
    it('should add a new file', async () => {
      const bundle = await ZipBundle.createEmpty();
      const data = new TextEncoder().encode('new content').buffer;
      
      await bundle.addFile(
        { path: '/new.js', contentType: 'application/javascript' },
        data
      );
      
      expect(bundle.hasFile('/new.js')).toBe(true);
      const fileData = await bundle.getFileData('/new.js');
      expect(new TextDecoder().decode(fileData!)).toBe('new content');
    });

    it('should throw when adding duplicate file without replace', async () => {
      const bundle = await ZipBundle.createEmpty();
      const data = new ArrayBuffer(10);
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        data
      );
      
      await expect(
        bundle.addFile(
          { path: '/test.js', contentType: 'text/javascript' },
          data
        )
      ).rejects.toThrow('already exists');
    });

    it('should replace file with replace option', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('original').buffer
      );
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('replaced').buffer,
        { replace: true }
      );
      
      const data = await bundle.getFileData('/test.js');
      expect(new TextDecoder().decode(data!)).toBe('replaced');
    });

    it('should update an existing file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('original').buffer
      );
      
      await bundle.updateFile(
        '/test.js',
        new TextEncoder().encode('updated').buffer
      );
      
      const data = await bundle.getFileData('/test.js');
      expect(new TextDecoder().decode(data!)).toBe('updated');
    });

    it('should throw when updating non-existent file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await expect(
        bundle.updateFile('/missing.js', new ArrayBuffer(10))
      ).rejects.toThrow('not found');
    });

    it('should remove a file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new ArrayBuffer(10)
      );
      
      expect(bundle.hasFile('/test.js')).toBe(true);
      
      await bundle.removeFile('/test.js');
      
      expect(bundle.hasFile('/test.js')).toBe(false);
    });

    it('should throw when removing non-existent file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await expect(bundle.removeFile('/missing.js')).rejects.toThrow('not found');
    });
  });

  describe('Entrypoint Modification Methods', () => {
    it('should set an entrypoint', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      
      bundle.setEntrypoint('main', '/main.js');
      
      expect(bundle.getEntrypoint('main')).toBe('/main.js');
    });

    it('should throw when setting entrypoint to non-existent file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      expect(() => bundle.setEntrypoint('main', '/missing.js')).toThrow('not found');
    });

    it('should remove an entrypoint', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      
      bundle.setEntrypoint('main', '/main.js');
      bundle.removeEntrypoint('main');
      
      expect(bundle.hasEntrypoint('main')).toBe(false);
    });

    it('should throw when removing non-existent entrypoint', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      expect(() => bundle.removeEntrypoint('missing')).toThrow('not found');
    });
  });

  describe('Validation Methods', () => {
    it('should validate a valid bundle', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(10)
      );
      
      const result = bundle.validate();
      expect(result.valid).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('should check if bundle is valid', async () => {
      const bundle = await ZipBundle.createEmpty();
      expect(bundle.isValid()).toBe(true);
    });
  });

  describe('Compression Methods', () => {
    it('should check if a file is compressed', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/compressed.js', contentType: 'text/javascript' },
        new ArrayBuffer(100),
        { compress: true }
      );
      
      await bundle.addFile(
        { path: '/uncompressed.js', contentType: 'text/javascript' },
        new ArrayBuffer(100),
        { compress: false }
      );
      
      expect(bundle.isFileCompressed('/compressed.js')).toBe(true);
      expect(bundle.isFileCompressed('/uncompressed.js')).toBe(false);
    });

    it('should throw when checking compression of non-existent file', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      expect(() => bundle.isFileCompressed('/missing.js')).toThrow('not found');
    });

    it('should get uncompressed size', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        {
          path: '/test.js',
          contentType: 'text/javascript',
          uncompressedSize: 1000,
        },
        new ArrayBuffer(500),
        { compress: true }
      );
      
      expect(bundle.getUncompressedSize('/test.js')).toBe(1000);
    });
  });

  describe('Utility Methods', () => {
    it('should get bundle info', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/main.js', contentType: 'application/javascript' },
        new ArrayBuffer(100)
      );
      
      bundle.setEntrypoint('main', '/main.js');
      
      const info = bundle.getBundleInfo();
      expect(info.version).toBe(1);
      expect(info.fileCount).toBe(1);
      expect(info.entrypoints).toEqual(['main']);
    });

    it('should estimate bundle size', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/file1.js', contentType: 'text/javascript' },
        new ArrayBuffer(1000)
      );
      
      await bundle.addFile(
        { path: '/file2.js', contentType: 'text/javascript' },
        new ArrayBuffer(2000)
      );
      
      const size = bundle.estimateBundleSize();
      expect(size).toBeGreaterThan(3000); // Files + manifest + overhead
    });

    it('should clone the bundle', async () => {
      const original = await ZipBundle.createEmpty();
      
      await original.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('content').buffer
      );
      
      original.setEntrypoint('main', '/test.js');
      
      const clone = await original.clone();
      
      expect(clone).not.toBe(original);
      expect(clone.hasFile('/test.js')).toBe(true);
      expect(clone.getEntrypoint('main')).toBe('/test.js');
      
      // Verify deep copy
      await original.removeFile('/test.js');
      expect(clone.hasFile('/test.js')).toBe(true);
    });
  });

  describe('Serialization Methods', () => {
    it('should serialize to ArrayBuffer', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('test content').buffer
      );
      
      const arrayBuffer = await bundle.toArrayBuffer();
      expect(arrayBuffer).toBeInstanceOf(ArrayBuffer);
      expect(arrayBuffer.byteLength).toBeGreaterThan(0);
    });

    it('should convert to Buffer in Node.js environment', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      await bundle.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new ArrayBuffer(100)
      );
      
      const buffer = await bundle.toBuffer();
      expect(buffer).toBeInstanceOf(Buffer);
      expect(buffer.byteLength).toBeGreaterThan(100);
    });
  });

  describe('Static Factory Methods', () => {
    it('should create an empty bundle', async () => {
      const bundle = await ZipBundle.createEmpty();
      
      expect(bundle).toBeInstanceOf(ZipBundle);
      expect(bundle.getFileCount()).toBe(0);
      expect(bundle.manifest.version).toBe(1);
    });

    it('should create an empty bundle with custom version', async () => {
      const bundle = await ZipBundle.createEmpty({ version: 2 });
      
      expect(bundle.manifest.version).toBe(2);
    });

    it('should create bundle from files', async () => {
      const files = new Map<string, ArrayBuffer>([
        ['file1.js', new TextEncoder().encode('content1').buffer],
        ['file2.css', new TextEncoder().encode('content2').buffer],
      ]);
      
      const contentTypes = new Map<string, string>([
        ['file1.js', 'application/javascript'],
        ['file2.css', 'text/css'],
      ]);
      
      const bundle = await ZipBundle.fromFiles(files, { contentTypes });
      
      expect(bundle.getFileCount()).toBe(2);
      expect(bundle.getFile('/file1.js')?.contentType).toBe('application/javascript');
      expect(bundle.getFile('/file2.css')?.contentType).toBe('text/css');
    });

    it('should parse bundle from ArrayBuffer', async () => {
      // Create a bundle and serialize it
      const original = await ZipBundle.createEmpty();
      await original.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new TextEncoder().encode('test content').buffer
      );
      original.setEntrypoint('main', '/test.js');
      
      const data = await original.toArrayBuffer();
      
      // Parse it back
      const parsed = await ZipBundle.parse(data);
      
      expect(parsed).toBeInstanceOf(ZipBundle);
      expect(parsed.hasFile('/test.js')).toBe(true);
      expect(parsed.getEntrypoint('main')).toBe('/test.js');
    });

    it('should parse bundle from Buffer', async () => {
      const original = await ZipBundle.createEmpty();
      await original.addFile(
        { path: '/test.js', contentType: 'text/javascript' },
        new ArrayBuffer(50)
      );
      
      const buffer = await original.toBuffer();
      // Convert Buffer to ArrayBuffer for parsing
      const arrayBuffer = new ArrayBuffer(buffer.byteLength);
      const view = new Uint8Array(arrayBuffer);
      view.set(new Uint8Array(buffer));
      
      const parsed = await ZipBundle.parse(arrayBuffer);
      
      expect(parsed).toBeInstanceOf(ZipBundle);
      expect(parsed.hasFile('/test.js')).toBe(true);
    });

    it('should throw when parsing invalid data', async () => {
      const invalidData = new ArrayBuffer(100);
      
      await expect(ZipBundle.parse(invalidData)).rejects.toThrow();
    });

    it('should enforce max size when parsing', async () => {
      const bundle = await ZipBundle.createEmpty();
      await bundle.addFile(
        { path: '/large.bin', contentType: 'application/octet-stream' },
        new ArrayBuffer(1000)
      );
      
      const data = await bundle.toArrayBuffer();
      
      await expect(
        ZipBundle.parse(data, { maxSize: 100 })
      ).rejects.toThrow('exceeds maximum allowed size');
    });
  });

  describe('Round-trip serialization', () => {
    it('should maintain data integrity through serialization', async () => {
      const original = await ZipBundle.createEmpty();
      
      // Add various files
      await original.addFile(
        { path: '/text.txt', contentType: 'text/plain' },
        new TextEncoder().encode('Hello, World!').buffer
      );
      
      await original.addFile(
        { path: '/data.json', contentType: 'application/json' },
        new TextEncoder().encode('{"test": true}').buffer
      );
      
      // Set entrypoints
      original.setEntrypoint('text', '/text.txt');
      original.setEntrypoint('data', '/data.json');
      
      // Serialize and parse
      const serialized = await original.toArrayBuffer();
      const restored = await ZipBundle.parse(serialized);
      
      // Verify files
      expect(restored.getFileCount()).toBe(2);
      const textData = await restored.getFileData('/text.txt');
      expect(new TextDecoder().decode(textData!)).toBe('Hello, World!');
      
      const jsonData = await restored.getFileData('/data.json');
      expect(new TextDecoder().decode(jsonData!)).toBe('{"test": true}');
      
      // Verify entrypoints
      expect(restored.getEntrypoint('text')).toBe('/text.txt');
      expect(restored.getEntrypoint('data')).toBe('/data.json');
    });
  });
});