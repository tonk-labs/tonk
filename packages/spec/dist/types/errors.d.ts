/**
 * Error types for Bundle Package File Format operations
 * Integrated with JSZip and Zod error handling
 */
/**
 * Base error class for all bundle-related errors
 */
export declare abstract class BundleError extends Error {
    /** Error code for programmatic handling */
    readonly code: string;
    /** Additional context about the error */
    readonly context?: Record<string, unknown>;
    constructor(message: string, code: string, context?: Record<string, unknown>);
}
/**
 * Error thrown when bundle parsing fails
 */
export declare class BundleParseError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static invalidZipFile(originalError: Error): BundleParseError;
    static missingManifest(): BundleParseError;
    static invalidManifestJson(originalError: Error): BundleParseError;
    static zipLoadFailed(originalError: Error): BundleParseError;
}
/**
 * Error thrown when bundle validation fails
 */
export declare class BundleValidationError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static missingRequiredField(field: string): BundleValidationError;
    static duplicateFilePath(path: string): BundleValidationError;
    static manifestFileInconsistency(manifestPath: string, zipHasFile: boolean): BundleValidationError;
    static zodSchemaError(zodError: any, context?: Record<string, unknown>): BundleValidationError;
    static invalidEntrypointPath(entrypoint: string, path: string): BundleValidationError;
    static invalidContentType(path: string, contentType: string): BundleValidationError;
}
/**
 * Error thrown when a requested file is not found in the bundle
 */
export declare class FileNotFoundError extends BundleError {
    constructor(path: string);
}
/**
 * Error thrown when an entrypoint is not found in the bundle
 */
export declare class EntrypointNotFoundError extends BundleError {
    constructor(entrypoint: string);
}
/**
 * Error thrown when ZIP operations fail
 */
export declare class ZipOperationError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static fileNotFoundInZip(path: string): ZipOperationError;
    static zipGenerationFailed(originalError: Error): ZipOperationError;
    static fileExtractionFailed(path: string, originalError: Error): ZipOperationError;
}
/**
 * Error thrown when bundle operations would exceed size limits
 */
export declare class BundleSizeError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static exceedsMaxSize(actualSize: number, maxSize: number): BundleSizeError;
}
/**
 * Error thrown when bundle format version is not supported
 */
export declare class UnsupportedVersionError extends BundleError {
    constructor(version: number, supportedVersions: number[]);
}
/**
 * Error thrown when schema validation fails
 */
export declare class SchemaValidationError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static fromZodError(zodError: any): SchemaValidationError;
}
/**
 * Error thrown when circular references are detected
 */
export declare class CircularReferenceError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static entrypointCycle(cycles: string[]): CircularReferenceError;
}
/**
 * Error thrown when validation context is invalid
 */
export declare class ValidationContextError extends BundleError {
    constructor(message: string, context?: Record<string, unknown>);
    static invalidRule(ruleId: string, originalError: Error): ValidationContextError;
}
/**
 * Enhanced error reporting utility
 */
export declare class EnhancedBundleError extends BundleError {
    readonly suggestions?: string[];
    readonly severity: 'error' | 'warning' | 'info';
    readonly recoverable: boolean;
    constructor(message: string, code: string, context?: Record<string, unknown>, options?: {
        suggestions?: string[];
        severity?: 'error' | 'warning' | 'info';
        recoverable?: boolean;
    });
    /**
     * Get a detailed error report including context and suggestions
     */
    getDetailedReport(): string;
}
//# sourceMappingURL=errors.d.ts.map