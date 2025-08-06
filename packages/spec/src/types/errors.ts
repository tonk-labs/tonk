/**
 * Error types for Bundle Package File Format operations
 * Integrated with JSZip and Zod error handling
 */

/**
 * Base error class for all bundle-related errors
 */
export abstract class BundleError extends Error {
  /** Error code for programmatic handling */
  public readonly code: string;

  /** Additional context about the error */
  public readonly context?: Record<string, unknown>;

  constructor(
    message: string,
    code: string,
    context?: Record<string, unknown>
  ) {
    super(message);
    this.name = this.constructor.name;
    this.code = code;
    this.context = context;

    // Maintain proper stack trace
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, this.constructor);
    }
  }
}

/**
 * Error thrown when bundle parsing fails
 */
export class BundleParseError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'BUNDLE_PARSE_ERROR', context);
  }

  static invalidZipFile(originalError: Error): BundleParseError {
    return new BundleParseError(`Invalid ZIP file: ${originalError.message}`, {
      originalError: originalError.message,
    });
  }

  static missingManifest(): BundleParseError {
    return new BundleParseError('Missing manifest.json file in ZIP archive', {
      expectedFile: 'manifest.json',
    });
  }

  static invalidManifestJson(originalError: Error): BundleParseError {
    return new BundleParseError(
      `Failed to parse manifest.json: ${originalError.message}`,
      { originalError: originalError.message, file: 'manifest.json' }
    );
  }

  static zipLoadFailed(originalError: Error): BundleParseError {
    return new BundleParseError(
      `Failed to load ZIP archive: ${originalError.message}`,
      { originalError: originalError.message }
    );
  }
}

/**
 * Error thrown when bundle validation fails
 */
export class BundleValidationError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'BUNDLE_VALIDATION_ERROR', context);
  }

  static missingRequiredField(field: string): BundleValidationError {
    return new BundleValidationError(`Missing required field: ${field}`, {
      field,
    });
  }

  static duplicateFilePath(path: string): BundleValidationError {
    return new BundleValidationError(`Duplicate file path: ${path}`, { path });
  }

  static manifestFileInconsistency(
    manifestPath: string,
    zipHasFile: boolean
  ): BundleValidationError {
    const message = zipHasFile
      ? `File ${manifestPath} exists in ZIP but not in manifest`
      : `File ${manifestPath} listed in manifest but not found in ZIP`;

    return new BundleValidationError(message, {
      path: manifestPath,
      inZip: zipHasFile,
      inManifest: !zipHasFile,
    });
  }

  static zodSchemaError(
    zodError: any,
    context?: Record<string, unknown>
  ): BundleValidationError {
    return new BundleValidationError(
      `Schema validation failed: ${zodError.message || 'Invalid data structure'}`,
      { zodError: zodError.toString(), ...context }
    );
  }

  static invalidEntrypointPath(
    entrypoint: string,
    path: string
  ): BundleValidationError {
    return new BundleValidationError(
      `Entrypoint "${entrypoint}" references non-existent file: ${path}`,
      { entrypoint, path }
    );
  }

  static invalidContentType(
    path: string,
    contentType: string
  ): BundleValidationError {
    return new BundleValidationError(
      `Invalid content type for ${path}: "${contentType}" is not a valid MIME type`,
      { path, contentType }
    );
  }
}

/**
 * Error thrown when a requested file is not found in the bundle
 */
export class FileNotFoundError extends BundleError {
  constructor(path: string) {
    super(`File not found: ${path}`, 'FILE_NOT_FOUND', { path });
  }
}

/**
 * Error thrown when an entrypoint is not found in the bundle
 */
export class EntrypointNotFoundError extends BundleError {
  constructor(entrypoint: string) {
    super(`Entrypoint not found: ${entrypoint}`, 'ENTRYPOINT_NOT_FOUND', {
      entrypoint,
    });
  }
}

/**
 * Error thrown when ZIP operations fail
 */
export class ZipOperationError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'ZIP_OPERATION_ERROR', context);
  }

  static fileNotFoundInZip(path: string): ZipOperationError {
    return new ZipOperationError(`File not found in ZIP archive: ${path}`, {
      path,
    });
  }

  static zipGenerationFailed(originalError: Error): ZipOperationError {
    return new ZipOperationError(
      `Failed to generate ZIP archive: ${originalError.message}`,
      { originalError: originalError.message }
    );
  }

  static fileExtractionFailed(
    path: string,
    originalError: Error
  ): ZipOperationError {
    return new ZipOperationError(
      `Failed to extract file ${path} from ZIP: ${originalError.message}`,
      { path, originalError: originalError.message }
    );
  }
}

/**
 * Error thrown when bundle operations would exceed size limits
 */
export class BundleSizeError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'BUNDLE_SIZE_ERROR', context);
  }

  static exceedsMaxSize(actualSize: number, maxSize: number): BundleSizeError {
    return new BundleSizeError(
      `Bundle size ${actualSize} bytes exceeds maximum allowed size ${maxSize} bytes`,
      { actualSize, maxSize }
    );
  }
}

/**
 * Error thrown when bundle format version is not supported
 */
export class UnsupportedVersionError extends BundleError {
  constructor(version: number, supportedVersions: number[]) {
    super(
      `Unsupported bundle format version: ${version}. Supported versions: ${supportedVersions.join(', ')}`,
      'UNSUPPORTED_VERSION',
      { version, supportedVersions }
    );
  }
}

/**
 * Error thrown when schema validation fails
 */
export class SchemaValidationError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'SCHEMA_VALIDATION_ERROR', context);
  }

  static fromZodError(zodError: any): SchemaValidationError {
    const issues = zodError.issues || [];
    const firstIssue = issues[0];
    const message = firstIssue?.message || 'Schema validation failed';
    const path = firstIssue?.path?.join('.') || 'unknown';

    return new SchemaValidationError(
      `Schema validation failed at ${path}: ${message}`,
      {
        zodError: zodError.toString(),
        path,
        issues: issues.length,
        firstIssue: firstIssue?.message,
      }
    );
  }
}

/**
 * Error thrown when circular references are detected
 */
export class CircularReferenceError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'CIRCULAR_REFERENCE_ERROR', context);
  }

  static entrypointCycle(cycles: string[]): CircularReferenceError {
    return new CircularReferenceError(
      `Circular entrypoint references detected: ${cycles.join(', ')}`,
      { cycles, count: cycles.length }
    );
  }
}

/**
 * Error thrown when validation context is invalid
 */
export class ValidationContextError extends BundleError {
  constructor(message: string, context?: Record<string, unknown>) {
    super(message, 'VALIDATION_CONTEXT_ERROR', context);
  }

  static invalidRule(
    ruleId: string,
    originalError: Error
  ): ValidationContextError {
    return new ValidationContextError(
      `Custom validation rule "${ruleId}" failed: ${originalError.message}`,
      {
        ruleId,
        originalError: originalError.message,
        stack: originalError.stack,
      }
    );
  }
}

/**
 * Enhanced error reporting utility
 */
export class EnhancedBundleError extends BundleError {
  public readonly suggestions?: string[];
  public readonly severity: 'error' | 'warning' | 'info';
  public readonly recoverable: boolean;

  constructor(
    message: string,
    code: string,
    context?: Record<string, unknown>,
    options?: {
      suggestions?: string[];
      severity?: 'error' | 'warning' | 'info';
      recoverable?: boolean;
    }
  ) {
    super(message, code, context);
    this.suggestions = options?.suggestions;
    this.severity = options?.severity || 'error';
    this.recoverable = options?.recoverable || false;
  }

  /**
   * Get a detailed error report including context and suggestions
   */
  getDetailedReport(): string {
    const lines: string[] = [];

    lines.push(`${this.severity.toUpperCase()}: ${this.message}`);
    lines.push(`Code: ${this.code}`);

    if (this.context && Object.keys(this.context).length > 0) {
      lines.push('Context:');
      for (const [key, value] of Object.entries(this.context)) {
        lines.push(`  ${key}: ${JSON.stringify(value)}`);
      }
    }

    if (this.suggestions && this.suggestions.length > 0) {
      lines.push('Suggestions:');
      for (const suggestion of this.suggestions) {
        lines.push(`  • ${suggestion}`);
      }
    }

    if (this.stack) {
      lines.push('\nStack trace:');
      lines.push(this.stack);
    }

    return lines.join('\n');
  }
}
