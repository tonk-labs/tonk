import { describe, it, expect } from 'vitest';
import type {
  BundleManifest,
  BundleFile,
  BundleInfo,
  AddFileOptions,
  CreateBundleOptions,
  SerializationOptions,
  ParseOptions,
  BundleVersion,
  EntrypointMap,
} from '../../types/bundle.js';
import type { ValidationResult } from '../../types/validation.js';

describe('Bundle Types', () => {
  describe('BundleManifest', () => {
    it('should accept valid manifest structure', () => {
      const manifest: BundleManifest = {
        version: 1,
        files: [
          {
            path: 'index.js',
            length: 100,
            contentType: 'application/javascript',
          },
        ],
        entrypoints: {
          main: 'index.js',
        },
      };

      expect(manifest.version).toBe(1);
      expect(manifest.files).toHaveLength(1);
      expect(manifest.entrypoints.main).toBe('index.js');
    });

    it('should accept manifest with optional fields', () => {
      const manifest: BundleManifest = {
        version: 1,
        name: 'My Bundle',
        description: 'A test bundle',
        createdAt: '2024-01-01T00:00:00Z',
        files: [],
        entrypoints: {},
        metadata: {
          author: 'Test Author',
          custom: 'value',
        },
      };

      expect(manifest.name).toBe('My Bundle');
      expect(manifest.description).toBe('A test bundle');
      expect(manifest.createdAt).toBe('2024-01-01T00:00:00Z');
      expect(manifest.metadata).toBeDefined();
      expect(manifest.metadata?.author).toBe('Test Author');
    });
  });

  describe('BundleFile', () => {
    it('should accept valid file structure', () => {
      const file: BundleFile = {
        path: 'src/main.js',
        length: 500,
        contentType: 'application/javascript',
      };

      expect(file.path).toBe('src/main.js');
      expect(file.length).toBe(500);
      expect(file.contentType).toBe('application/javascript');
    });

    it('should accept file with compressed flag', () => {
      const file: BundleFile = {
        path: 'data.json',
        length: 1000,
        contentType: 'application/json',
        compressed: true,
      };

      expect(file.compressed).toBe(true);
    });

    it('should accept file with all optional fields', () => {
      const file: BundleFile = {
        path: 'large.json',
        length: 500,
        contentType: 'application/json',
        compressed: true,
        uncompressedSize: 2000,
        lastModified: '2024-01-01T12:00:00Z',
      };

      expect(file.uncompressedSize).toBe(2000);
      expect(file.lastModified).toBe('2024-01-01T12:00:00Z');
    });
  });

  describe('BundleInfo', () => {
    it('should accept valid bundle info structure', () => {
      const info: BundleInfo = {
        version: 1,
        fileCount: 10,
        totalSize: 50000,
        compressedFiles: 3,
        entrypoints: ['main', 'worker'],
        uncompressedSize: 100000,
      };

      expect(info.version).toBe(1);
      expect(info.fileCount).toBe(10);
      expect(info.totalSize).toBe(50000);
      expect(info.compressedFiles).toBe(3);
      expect(info.entrypoints).toHaveLength(2);
      expect(info.uncompressedSize).toBe(100000);
    });

    it('should accept bundle info with optional fields', () => {
      const info: BundleInfo = {
        version: 1,
        name: 'Test Bundle',
        fileCount: 5,
        totalSize: 10000,
        compressedFiles: 2,
        entrypoints: [],
        uncompressedSize: 15000,
        createdAt: '2024-01-01T00:00:00Z',
      };

      expect(info.name).toBe('Test Bundle');
      expect(info.createdAt).toBe('2024-01-01T00:00:00Z');
    });
  });

  describe('BundleInfo with ValidationResult', () => {
    it('should use ValidationResult from validation module', () => {
      const result: ValidationResult = {
        valid: true,
        messages: [],
        errors: [],
        warnings: [],
        info: [],
      };

      expect(result.valid).toBe(true);
      expect(result.messages).toHaveLength(0);
      expect(result.errors).toHaveLength(0);
      expect(result.warnings).toHaveLength(0);
      expect(result.info).toHaveLength(0);
    });
  });

  describe('AddFileOptions', () => {
    it('should accept valid add file options', () => {
      const options: AddFileOptions = {
        contentType: 'text/plain',
        compress: true,
        replace: true,
        lastModified: '2024-01-01T12:00:00Z',
      };

      expect(options.contentType).toBe('text/plain');
      expect(options.compress).toBe(true);
      expect(options.replace).toBe(true);
      expect(options.lastModified).toBe('2024-01-01T12:00:00Z');
    });
  });

  describe('CreateBundleOptions', () => {
    it('should accept valid bundle creation options', () => {
      const options: CreateBundleOptions = {
        version: 1,
        name: 'My Bundle',
        description: 'A test bundle',
        metadata: {
          author: 'Test',
          license: 'MIT',
        },
      };

      expect(options.version).toBe(1);
      expect(options.name).toBe('My Bundle');
      expect(options.description).toBe('A test bundle');
      expect(options.metadata?.author).toBe('Test');
    });
  });

  describe('SerializationOptions', () => {
    it('should accept valid serialization options', () => {
      const options: SerializationOptions = {
        compressionLevel: 9,
        useZip64: true,
        comment: 'Generated bundle',
      };

      expect(options.compressionLevel).toBe(9);
      expect(options.useZip64).toBe(true);
      expect(options.comment).toBe('Generated bundle');
    });
  });

  describe('ParseOptions', () => {
    it('should accept valid parse options', () => {
      const options: ParseOptions = {
        strictValidation: true,
        validateFileReferences: true,
        maxSize: 100 * 1024 * 1024, // 100MB
      };

      expect(options.strictValidation).toBe(true);
      expect(options.validateFileReferences).toBe(true);
      expect(options.maxSize).toBe(104857600);
    });
  });

  describe('Type aliases', () => {
    it('should accept BundleVersion', () => {
      const version: BundleVersion = 1;
      expect(version).toBe(1);
    });

    it('should accept EntrypointMap', () => {
      const entrypoints: EntrypointMap = {
        main: 'src/index.js',
        worker: 'src/worker.js',
      };

      expect(entrypoints.main).toBe('src/index.js');
      expect(entrypoints.worker).toBe('src/worker.js');
    });
  });
});