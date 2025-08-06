import { describe, it, expect } from 'vitest';
import type { 
  ValidationOptions, 
  ValidationResult, 
  ValidationMessage,
  ValidationRule,
  ValidationContext,
} from '../../types/validation.js';
import { 
  ValidationSeverity,
  ValidationRules,
  ValidationResultBuilder,
} from '../../types/validation.js';

describe('Validation Types', () => {
  describe('ValidationOptions', () => {
    it('should accept minimal validation options', () => {
      const options: ValidationOptions = {};
      expect(options).toBeDefined();
    });

    it('should accept validation options with all fields', () => {
      const options: ValidationOptions = {
        includeWarnings: true,
        includeInfo: true,
        failFast: false,
        maxBundleSize: 100 * 1024 * 1024, // 100MB
        maxFileCount: 1000,
        strictMimeTypes: true,
        customRules: [],
      };

      expect(options.includeWarnings).toBe(true);
      expect(options.includeInfo).toBe(true);
      expect(options.failFast).toBe(false);
      expect(options.maxBundleSize).toBe(104857600);
      expect(options.maxFileCount).toBe(1000);
      expect(options.strictMimeTypes).toBe(true);
      expect(options.customRules).toHaveLength(0);
    });

    it('should accept partial validation options', () => {
      const options: ValidationOptions = {
        maxBundleSize: 50 * 1024 * 1024,
        includeWarnings: false,
      };

      expect(options.maxBundleSize).toBe(52428800);
      expect(options.includeWarnings).toBe(false);
      expect(options.includeInfo).toBeUndefined();
    });
  });

  describe('ValidationResult', () => {
    it('should accept valid validation result', () => {
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

    it('should accept validation result with messages', () => {
      const errorMessage: ValidationMessage = {
        severity: ValidationSeverity.ERROR,
        message: 'File not found',
        code: 'FILE_NOT_FOUND',
        context: { path: 'missing.js' },
        filePath: 'missing.js',
      };

      const warningMessage: ValidationMessage = {
        severity: ValidationSeverity.WARNING,
        message: 'Large file detected',
        code: 'LARGE_FILE',
        suggestion: 'Consider compressing this file',
      };

      const result: ValidationResult = {
        valid: false,
        messages: [errorMessage, warningMessage],
        errors: [errorMessage],
        warnings: [warningMessage],
        info: [],
      };

      expect(result.valid).toBe(false);
      expect(result.messages).toHaveLength(2);
      expect(result.errors).toHaveLength(1);
      expect(result.warnings).toHaveLength(1);
    });
  });

  describe('ValidationRule', () => {
    it('should accept valid validation rule', () => {
      const rule: ValidationRule = {
        id: 'custom-rule',
        description: 'Custom validation rule',
        validate: (_context: ValidationContext) => {
          return [];
        },
      };

      expect(rule.id).toBe('custom-rule');
      expect(rule.description).toBe('Custom validation rule');
      expect(rule.validate).toBeDefined();
    });
  });

  describe('ValidationResultBuilder', () => {
    it('should build empty validation result', () => {
      const builder = new ValidationResultBuilder();
      const result = builder.build();

      expect(result.valid).toBe(true);
      expect(result.messages).toHaveLength(0);
      expect(result.errors).toHaveLength(0);
      expect(result.warnings).toHaveLength(0);
      expect(result.info).toHaveLength(0);
    });

    it('should build validation result with errors', () => {
      const builder = new ValidationResultBuilder();
      builder
        .addError('Missing file', 'MISSING_FILE', { path: 'test.js' })
        .addError('Invalid version', 'INVALID_VERSION');

      const result = builder.build();
      expect(result.valid).toBe(false);
      expect(result.errors).toHaveLength(2);
      expect(builder.hasErrors()).toBe(true);
      expect(builder.getMessageCount(ValidationSeverity.ERROR)).toBe(2);
    });

    it('should build validation result with mixed messages', () => {
      const builder = new ValidationResultBuilder();
      builder
        .addError('Critical error', 'CRITICAL')
        .addWarning('Performance warning', 'PERF_WARNING')
        .addInfo('Bundle optimized', 'OPTIMIZED');

      const result = builder.build();
      expect(result.valid).toBe(false);
      expect(result.messages).toHaveLength(3);
      expect(result.errors).toHaveLength(1);
      expect(result.warnings).toHaveLength(1);
      expect(result.info).toHaveLength(1);
      expect(builder.getMessageCount()).toBe(3);
    });

    it('should support method chaining', () => {
      const builder = new ValidationResultBuilder();
      const result = builder
        .addError('Error 1', 'ERR1')
        .addWarning('Warning 1', 'WARN1')
        .addInfo('Info 1', 'INFO1')
        .build();

      expect(result.messages).toHaveLength(3);
    });
  });

  describe('ValidationRules constants', () => {
    it('should have predefined rule identifiers', () => {
      expect(ValidationRules.REQUIRED_FIELDS).toBe('required-fields');
      expect(ValidationRules.UNIQUE_FILE_PATHS).toBe('unique-file-paths');
      expect(ValidationRules.VALID_ENTRYPOINTS).toBe('valid-entrypoints');
      expect(ValidationRules.VALID_MIME_TYPES).toBe('valid-mime-types');
      expect(ValidationRules.BUNDLE_SIZE_LIMIT).toBe('bundle-size-limit');
      expect(ValidationRules.FILE_COUNT_LIMIT).toBe('file-count-limit');
      expect(ValidationRules.VALID_VERSION).toBe('valid-version');
      expect(ValidationRules.MANIFEST_ZIP_CONSISTENCY).toBe('manifest-zip-consistency');
      expect(ValidationRules.ZOD_SCHEMA_VALIDATION).toBe('zod-schema-validation');
    });
  });
});