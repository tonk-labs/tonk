/**
 * Validation types and interfaces for Bundle Package File Format
 * Integrates with Zod for schema validation and error handling
 */
/**
 * Severity level for validation messages
 */
export declare enum ValidationSeverity {
    ERROR = "error",
    WARNING = "warning",
    INFO = "info"
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
    manifest: any;
    /** The ZIP archive data (if available) */
    zipData?: ArrayBuffer;
    /** Validation options */
    options: ValidationOptions;
    /** Helper function to create validation messages */
    createMessage: (severity: ValidationSeverity, message: string, code: string, context?: Record<string, unknown>) => ValidationMessage;
}
/**
 * Pre-defined validation rule identifiers
 */
export declare const ValidationRules: {
    readonly REQUIRED_FIELDS: "required-fields";
    readonly UNIQUE_FILE_PATHS: "unique-file-paths";
    readonly VALID_ENTRYPOINTS: "valid-entrypoints";
    readonly VALID_MIME_TYPES: "valid-mime-types";
    readonly BUNDLE_SIZE_LIMIT: "bundle-size-limit";
    readonly FILE_COUNT_LIMIT: "file-count-limit";
    readonly VALID_VERSION: "valid-version";
    readonly MANIFEST_ZIP_CONSISTENCY: "manifest-zip-consistency";
    readonly ZOD_SCHEMA_VALIDATION: "zod-schema-validation";
};
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
export declare class ValidationResultBuilder {
    private messages;
    /**
     * Add an error message
     */
    addError(message: string, code: string, context?: Record<string, unknown>, filePath?: string): this;
    /**
     * Add a warning message
     */
    addWarning(message: string, code: string, context?: Record<string, unknown>, filePath?: string): this;
    /**
     * Add an info message
     */
    addInfo(message: string, code: string, context?: Record<string, unknown>, filePath?: string): this;
    /**
     * Build the final validation result
     */
    build(): ValidationResult;
    /**
     * Check if there are any errors
     */
    hasErrors(): boolean;
    /**
     * Get the number of messages by severity
     */
    getMessageCount(severity?: ValidationSeverity): number;
    /**
     * Add validation messages from a Zod error with enhanced context
     * This method helps integrate Zod validation errors with detailed error reporting
     */
    addZodError(zodError: any, // Using any to avoid requiring Zod as a dependency in types
    baseMessage?: string, preserveStack?: boolean): this;
    /**
     * Add a validation message with enhanced error context and suggestions
     */
    addEnhancedMessage(severity: ValidationSeverity, message: string, code: string, context?: Record<string, unknown>, filePath?: string, suggestion?: string): this;
    /**
     * Add an error with stack trace preservation
     */
    addErrorWithStack(message: string, code: string, originalError?: Error, context?: Record<string, unknown>, filePath?: string): this;
}
//# sourceMappingURL=validation.d.ts.map