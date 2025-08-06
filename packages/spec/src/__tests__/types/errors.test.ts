import { describe, it, expect } from 'vitest';
import {
  BundleError,
  BundleParseError,
  BundleValidationError,
  FileNotFoundError,
  EntrypointNotFoundError,
  ZipOperationError,
  BundleSizeError,
  UnsupportedVersionError,
  SchemaValidationError,
} from '../../types/errors.js';

describe('Error Types', () => {
  describe('BundleError', () => {
    it('should create a base bundle error with code and context', () => {
      class TestError extends BundleError {
        constructor() {
          super('Test error message', 'TEST_ERROR', { foo: 'bar' });
        }
      }
      
      const error = new TestError();
      expect(error).toBeInstanceOf(Error);
      expect(error).toBeInstanceOf(BundleError);
      expect(error.message).toBe('Test error message');
      expect(error.name).toBe('TestError');
      expect(error.code).toBe('TEST_ERROR');
      expect(error.context).toEqual({ foo: 'bar' });
    });
  });

  describe('BundleParseError', () => {
    it('should create a parse error', () => {
      const error = new BundleParseError('Invalid manifest format');
      expect(error).toBeInstanceOf(Error);
      expect(error).toBeInstanceOf(BundleError);
      expect(error).toBeInstanceOf(BundleParseError);
      expect(error.message).toBe('Invalid manifest format');
      expect(error.name).toBe('BundleParseError');
      expect(error.code).toBe('BUNDLE_PARSE_ERROR');
    });

    it('should create error for invalid ZIP file', () => {
      const originalError = new Error('Not a ZIP file');
      const error = BundleParseError.invalidZipFile(originalError);
      expect(error.message).toBe('Invalid ZIP file: Not a ZIP file');
      expect(error.context?.originalError).toBe('Not a ZIP file');
    });

    it('should create error for missing manifest', () => {
      const error = BundleParseError.missingManifest();
      expect(error.message).toBe('Missing manifest.json file in ZIP archive');
      expect(error.context?.expectedFile).toBe('manifest.json');
    });

    it('should create error for invalid manifest JSON', () => {
      const originalError = new Error('Unexpected token');
      const error = BundleParseError.invalidManifestJson(originalError);
      expect(error.message).toBe('Failed to parse manifest.json: Unexpected token');
      expect(error.context?.file).toBe('manifest.json');
    });

    it('should create error for ZIP load failure', () => {
      const originalError = new Error('Corrupted ZIP');
      const error = BundleParseError.zipLoadFailed(originalError);
      expect(error.message).toBe('Failed to load ZIP archive: Corrupted ZIP');
    });
  });

  describe('BundleValidationError', () => {
    it('should create a validation error', () => {
      const error = new BundleValidationError('Validation failed');
      expect(error).toBeInstanceOf(BundleError);
      expect(error).toBeInstanceOf(BundleValidationError);
      expect(error.name).toBe('BundleValidationError');
      expect(error.code).toBe('BUNDLE_VALIDATION_ERROR');
    });

    it('should create error for missing required field', () => {
      const error = BundleValidationError.missingRequiredField('version');
      expect(error.message).toBe('Missing required field: version');
      expect(error.context?.field).toBe('version');
    });

    it('should create error for duplicate file path', () => {
      const error = BundleValidationError.duplicateFilePath('/src/index.js');
      expect(error.message).toBe('Duplicate file path: /src/index.js');
      expect(error.context?.path).toBe('/src/index.js');
    });

    it('should create error for manifest/ZIP inconsistency', () => {
      const error1 = BundleValidationError.manifestFileInconsistency('test.js', true);
      expect(error1.message).toBe('File test.js exists in ZIP but not in manifest');
      expect(error1.context?.inZip).toBe(true);
      expect(error1.context?.inManifest).toBe(false);

      const error2 = BundleValidationError.manifestFileInconsistency('test.js', false);
      expect(error2.message).toBe('File test.js listed in manifest but not found in ZIP');
      expect(error2.context?.inZip).toBe(false);
      expect(error2.context?.inManifest).toBe(true);
    });

    it('should create error for invalid entrypoint path', () => {
      const error = BundleValidationError.invalidEntrypointPath('main', 'missing.js');
      expect(error.message).toBe('Entrypoint "main" references non-existent file: missing.js');
      expect(error.context?.entrypoint).toBe('main');
      expect(error.context?.path).toBe('missing.js');
    });

    it('should create error for invalid content type', () => {
      const error = BundleValidationError.invalidContentType('test.js', 'not/a/mime');
      expect(error.message).toBe('Invalid content type for test.js: "not/a/mime" is not a valid MIME type');
      expect(error.context?.path).toBe('test.js');
      expect(error.context?.contentType).toBe('not/a/mime');
    });
  });

  describe('FileNotFoundError', () => {
    it('should create a file not found error', () => {
      const error = new FileNotFoundError('index.js');
      expect(error).toBeInstanceOf(BundleError);
      expect(error).toBeInstanceOf(FileNotFoundError);
      expect(error.message).toBe('File not found: index.js');
      expect(error.name).toBe('FileNotFoundError');
      expect(error.code).toBe('FILE_NOT_FOUND');
      expect(error.context?.path).toBe('index.js');
    });
  });

  describe('EntrypointNotFoundError', () => {
    it('should create an entrypoint not found error', () => {
      const error = new EntrypointNotFoundError('main');
      expect(error).toBeInstanceOf(BundleError);
      expect(error).toBeInstanceOf(EntrypointNotFoundError);
      expect(error.message).toBe('Entrypoint not found: main');
      expect(error.name).toBe('EntrypointNotFoundError');
      expect(error.code).toBe('ENTRYPOINT_NOT_FOUND');
      expect(error.context?.entrypoint).toBe('main');
    });
  });

  describe('ZipOperationError', () => {
    it('should create a ZIP operation error', () => {
      const error = new ZipOperationError('ZIP operation failed');
      expect(error).toBeInstanceOf(BundleError);
      expect(error).toBeInstanceOf(ZipOperationError);
      expect(error.code).toBe('ZIP_OPERATION_ERROR');
    });

    it('should create error for file not found in ZIP', () => {
      const error = ZipOperationError.fileNotFoundInZip('missing.js');
      expect(error.message).toBe('File not found in ZIP archive: missing.js');
      expect(error.context?.path).toBe('missing.js');
    });

    it('should create error for ZIP generation failure', () => {
      const originalError = new Error('Out of memory');
      const error = ZipOperationError.zipGenerationFailed(originalError);
      expect(error.message).toBe('Failed to generate ZIP archive: Out of memory');
    });

    it('should create error for file extraction failure', () => {
      const originalError = new Error('Corrupted data');
      const error = ZipOperationError.fileExtractionFailed('test.js', originalError);
      expect(error.message).toBe('Failed to extract file test.js from ZIP: Corrupted data');
      expect(error.context?.path).toBe('test.js');
    });
  });

  describe('BundleSizeError', () => {
    it('should create a bundle size error', () => {
      const error = new BundleSizeError('Bundle too large');
      expect(error).toBeInstanceOf(BundleError);
      expect(error.code).toBe('BUNDLE_SIZE_ERROR');
    });

    it('should create error for exceeding max size', () => {
      const error = BundleSizeError.exceedsMaxSize(200_000_000, 100_000_000);
      expect(error.message).toBe('Bundle size 200000000 bytes exceeds maximum allowed size 100000000 bytes');
      expect(error.context?.actualSize).toBe(200_000_000);
      expect(error.context?.maxSize).toBe(100_000_000);
    });
  });

  describe('UnsupportedVersionError', () => {
    it('should create an unsupported version error', () => {
      const error = new UnsupportedVersionError(3, [1, 2]);
      expect(error).toBeInstanceOf(BundleError);
      expect(error.message).toBe('Unsupported bundle format version: 3. Supported versions: 1, 2');
      expect(error.code).toBe('UNSUPPORTED_VERSION');
      expect(error.context?.version).toBe(3);
      expect(error.context?.supportedVersions).toEqual([1, 2]);
    });
  });

  describe('SchemaValidationError', () => {
    it('should create a schema validation error', () => {
      const error = new SchemaValidationError('Schema validation failed');
      expect(error).toBeInstanceOf(BundleError);
      expect(error.code).toBe('SCHEMA_VALIDATION_ERROR');
    });

    it('should create error from Zod error', () => {
      const zodError = {
        issues: [{
          message: 'Required',
          path: ['manifest', 'version'],
        }],
        toString: () => 'ZodError: Required at manifest.version',
      };
      
      const error = SchemaValidationError.fromZodError(zodError);
      expect(error.message).toBe('Schema validation failed at manifest.version: Required');
      expect(error.context?.path).toBe('manifest.version');
      expect(error.context?.issues).toBe(1);
      expect(error.context?.firstIssue).toBe('Required');
    });

    it('should handle Zod error without issues', () => {
      const zodError = {
        toString: () => 'ZodError: Invalid data',
      };
      
      const error = SchemaValidationError.fromZodError(zodError);
      expect(error.message).toBe('Schema validation failed at unknown: Schema validation failed');
    });
  });

  describe('Error inheritance', () => {
    it('should maintain proper instanceof relationships', () => {
      const parseError = new BundleParseError('Parse failed');
      const validationError = new BundleValidationError('Validation failed');
      const fileNotFoundError = new FileNotFoundError('missing.js');
      const zipError = new ZipOperationError('ZIP failed');

      // All should be instances of Error and BundleError
      [parseError, validationError, fileNotFoundError, zipError].forEach(error => {
        expect(error).toBeInstanceOf(Error);
        expect(error).toBeInstanceOf(BundleError);
      });

      // But not instances of each other's specific types
      expect(parseError).not.toBeInstanceOf(BundleValidationError);
      expect(validationError).not.toBeInstanceOf(BundleParseError);
      expect(fileNotFoundError).not.toBeInstanceOf(BundleParseError);
      expect(zipError).not.toBeInstanceOf(FileNotFoundError);
    });
  });
});