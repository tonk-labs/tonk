/**
 * Validation types and interfaces for Bundle Package File Format
 * Integrates with Zod for schema validation and error handling
 */

/**
 * Severity level for validation messages
 */
export enum ValidationSeverity {
  ERROR = 'error',
  WARNING = 'warning',
  INFO = 'info',
}

/**
 * A single validation message
 */
export interface ValidationMessage {
  /** Severity level of the message */
  severity: ValidationSeverity;

  /** Human-readable message */
  message: string;

  /** Machine-readable error code */
  code: string;

  /** Context information about the validation issue */
  context?: Record<string, unknown>;

  /** Suggested fix for the issue (if available) */
  suggestion?: string;

  /** File path associated with the issue (if applicable) */
  filePath?: string;
}

/**
 * Result of a validation operation
 */
export interface ValidationResult {
  /** Whether the validation passed (no errors) */
  valid: boolean;

  /** All validation messages */
  messages: ValidationMessage[];

  /** Quick access to error messages */
  errors: ValidationMessage[];

  /** Quick access to warning messages */
  warnings: ValidationMessage[];

  /** Quick access to info messages */
  info: ValidationMessage[];
}

/**
 * Options for validation operations
 */
export interface ValidationOptions {
  /** Whether to include warnings in the result */
  includeWarnings?: boolean;

  /** Whether to include info messages in the result */
  includeInfo?: boolean;

  /** Whether to stop validation on first error */
  failFast?: boolean;

  /** Maximum bundle size to allow (in bytes) */
  maxBundleSize?: number;

  /** Maximum number of files to allow */
  maxFileCount?: number;

  /** Whether to validate MIME types strictly */
  strictMimeTypes?: boolean;

  /** Custom validation rules to apply */
  customRules?: ValidationRule[];
}

/**
 * Custom validation rule interface
 */
export interface ValidationRule {
  /** Unique identifier for the rule */
  id: string;

  /** Human-readable description of the rule */
  description: string;

  /** Function that performs the validation */
  validate: (context: ValidationContext) => ValidationMessage[];
}

/**
 * Context passed to validation rules
 */
export interface ValidationContext {
  /** The bundle manifest being validated */
  manifest: any; // Using any to avoid circular dependency

  /** The ZIP archive data (if available) */
  zipData?: ArrayBuffer;

  /** Validation options */
  options: ValidationOptions;

  /** Helper function to create validation messages */
  createMessage: (
    severity: ValidationSeverity,
    message: string,
    code: string,
    context?: Record<string, unknown>
  ) => ValidationMessage;
}

/**
 * Pre-defined validation rule identifiers
 */
export const ValidationRules = {
  REQUIRED_FIELDS: 'required-fields',
  UNIQUE_FILE_PATHS: 'unique-file-paths',
  VALID_ENTRYPOINTS: 'valid-entrypoints',
  VALID_MIME_TYPES: 'valid-mime-types',
  BUNDLE_SIZE_LIMIT: 'bundle-size-limit',
  FILE_COUNT_LIMIT: 'file-count-limit',
  VALID_VERSION: 'valid-version',
  MANIFEST_ZIP_CONSISTENCY: 'manifest-zip-consistency',
  ZOD_SCHEMA_VALIDATION: 'zod-schema-validation',
} as const;

/**
 * Utility type for converting Zod errors to ValidationMessages
 */
export interface ZodErrorContext {
  /** The path in the object where the error occurred */
  path: (string | number)[];

  /** The Zod error code */
  zodCode: string;

  /** Expected value or type */
  expected?: unknown;

  /** Actual value that failed validation */
  received?: unknown;
}

/**
 * Helper class for building validation results
 */
export class ValidationResultBuilder {
  private messages: ValidationMessage[] = [];

  /**
   * Add an error message
   */
  addError(
    message: string,
    code: string,
    context?: Record<string, unknown>,
    filePath?: string
  ): this {
    this.messages.push({
      severity: ValidationSeverity.ERROR,
      message,
      code,
      context,
      filePath,
    });
    return this;
  }

  /**
   * Add a warning message
   */
  addWarning(
    message: string,
    code: string,
    context?: Record<string, unknown>,
    filePath?: string
  ): this {
    this.messages.push({
      severity: ValidationSeverity.WARNING,
      message,
      code,
      context,
      filePath,
    });
    return this;
  }

  /**
   * Add an info message
   */
  addInfo(
    message: string,
    code: string,
    context?: Record<string, unknown>,
    filePath?: string
  ): this {
    this.messages.push({
      severity: ValidationSeverity.INFO,
      message,
      code,
      context,
      filePath,
    });
    return this;
  }

  /**
   * Build the final validation result
   */
  build(): ValidationResult {
    const errors = this.messages.filter(
      m => m.severity === ValidationSeverity.ERROR
    );
    const warnings = this.messages.filter(
      m => m.severity === ValidationSeverity.WARNING
    );
    const info = this.messages.filter(
      m => m.severity === ValidationSeverity.INFO
    );

    return {
      valid: errors.length === 0,
      messages: [...this.messages],
      errors,
      warnings,
      info,
    };
  }

  /**
   * Check if there are any errors
   */
  hasErrors(): boolean {
    return this.messages.some(m => m.severity === ValidationSeverity.ERROR);
  }

  /**
   * Get the number of messages by severity
   */
  getMessageCount(severity?: ValidationSeverity): number {
    if (severity) {
      return this.messages.filter(m => m.severity === severity).length;
    }
    return this.messages.length;
  }

  /**
   * Add validation messages from a Zod error with enhanced context
   * This method helps integrate Zod validation errors with detailed error reporting
   */
  addZodError(
    zodError: any, // Using any to avoid requiring Zod as a dependency in types
    baseMessage = 'Schema validation failed',
    preserveStack = true
  ): this {
    if (zodError && zodError.issues && Array.isArray(zodError.issues)) {
      // Process individual Zod issues with enhanced detail
      for (const issue of zodError.issues) {
        const path = issue.path?.join('.') || 'unknown';
        const enhancedMessage = `${baseMessage} at ${path}: ${issue.message}`;

        this.addError(enhancedMessage, ValidationRules.ZOD_SCHEMA_VALIDATION, {
          zodError: issue,
          path,
          received: issue.received,
          expected: issue.expected,
          code: issue.code,
          stack: preserveStack && zodError.stack ? zodError.stack : undefined,
          timestamp: new Date().toISOString(),
        });
      }
    } else {
      // Fallback for non-Zod errors or malformed Zod errors
      this.addError(baseMessage, ValidationRules.ZOD_SCHEMA_VALIDATION, {
        zodError: zodError?.toString() || 'Unknown error',
        stack: preserveStack && zodError?.stack ? zodError.stack : undefined,
        timestamp: new Date().toISOString(),
      });
    }
    return this;
  }

  /**
   * Add a validation message with enhanced error context and suggestions
   */
  addEnhancedMessage(
    severity: ValidationSeverity,
    message: string,
    code: string,
    context?: Record<string, unknown>,
    filePath?: string,
    suggestion?: string
  ): this {
    const enhancedContext = {
      ...context,
      timestamp: new Date().toISOString(),
      severity,
    };

    this.messages.push({
      severity,
      message,
      code,
      context: enhancedContext,
      filePath,
      suggestion,
    });
    return this;
  }

  /**
   * Add an error with stack trace preservation
   */
  addErrorWithStack(
    message: string,
    code: string,
    originalError?: Error,
    context?: Record<string, unknown>,
    filePath?: string
  ): this {
    const enhancedContext: Record<string, unknown> = {
      ...context,
      stack: originalError?.stack,
      errorName: originalError?.name,
      timestamp: new Date().toISOString(),
    };

    if (originalError?.stack) {
      enhancedContext.originalMessage = originalError.message;
    }

    this.addError(message, code, enhancedContext, filePath);
    return this;
  }
}
