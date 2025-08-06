import { ValidationResult, ValidationMessage, ValidationSeverity, ValidationOptions } from '../types/validation.js';
import { BundleManifestType } from './bundle.js';

/**
 * Validates a bundle manifest using Zod schema
 */
export declare function validateManifest(manifest: unknown, options?: Partial<ValidationOptions>): ValidationResult;
/**
 * Validates individual bundle file metadata
 */
export declare function validateBundleFile(file: unknown): ValidationResult;
/**
 * Validates entrypoint mappings
 */
export declare function validateEntrypoints(entrypoints: unknown): ValidationResult;
/**
 * Creates a validation message helper function
 */
export declare function createValidationMessage(severity: ValidationSeverity, message: string, code: string, context?: Record<string, unknown>, filePath?: string): ValidationMessage;
/**
 * Validates that a manifest is consistent with ZIP contents
 */
export declare function validateManifestZipConsistency(manifest: BundleManifestType, zipFileNames: string[]): ValidationResult;
//# sourceMappingURL=validation.d.ts.map