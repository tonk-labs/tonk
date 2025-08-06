import { BundleManifestType } from '../schemas/bundle.js';
import { ValidationResult, ValidationMessage, ValidationOptions } from '../types/validation.js';
import { default as JSZip } from 'jszip';

/**
 * Detect circular references in entrypoint definitions
 *
 * This function performs graph analysis to detect cycles where entrypoints
 * reference each other in a circular fashion.
 *
 * @param entrypoints - Record of entrypoint names to file paths
 * @returns Array of detected circular reference chains
 */
export declare function detectCircularEntrypointReferences(entrypoints: Record<string, string>): string[];
/**
 * Validate a parsed manifest object using Zod schemas
 *
 * @param manifestData - Raw manifest data to validate
 * @param strict - Whether to use strict validation mode
 * @returns ValidationResult with success/error information
 */
export declare function validateManifestData(manifestData: unknown, strict?: boolean): ValidationResult;
/**
 * Validate consistency between manifest declarations and ZIP contents
 *
 * This function performs comprehensive validation to ensure that:
 * - All files in the manifest exist in the ZIP
 * - All entrypoints reference existing files
 * - File metadata is consistent where possible
 *
 * @param zip - JSZip instance containing bundle data
 * @param manifest - Validated bundle manifest
 * @returns ValidationResult with detailed error information
 */
export declare function validateZipManifestConsistency(zip: JSZip, manifest: BundleManifestType): ValidationResult;
/**
 * Validate bundle size constraints
 *
 * @param zipSize - Size of the ZIP archive in bytes
 * @param manifest - Bundle manifest
 * @param maxSize - Maximum allowed size in bytes (optional)
 * @returns ValidationResult with size validation
 */
export declare function validateBundleSize(zipSize: number, manifest: BundleManifestType, maxSize?: number): ValidationResult;
/**
 * Perform comprehensive bundle validation
 *
 * This function combines all validation checks into a single comprehensive validation
 * with enhanced error reporting and stack trace preservation.
 *
 * @param zip - JSZip instance containing bundle data
 * @param manifest - Bundle manifest
 * @param zipSize - Size of the ZIP archive
 * @param options - Comprehensive validation options
 * @returns Combined validation result with detailed error information
 */
export declare function validateBundleComprehensive(zip: JSZip, manifest: BundleManifestType, zipSize: number, options?: ValidationOptions): ValidationResult;
/**
 * Perform bundle validation (simplified version for backward compatibility)
 *
 * @param zip - JSZip instance containing bundle data
 * @param manifest - Bundle manifest
 * @param zipSize - Size of the ZIP archive
 * @param options - Basic validation options
 * @returns Combined validation result
 */
export declare function validateBundle(zip: JSZip, manifest: BundleManifestType, zipSize: number, options?: {
    maxSize?: number;
    strictValidation?: boolean;
    validateFileReferences?: boolean;
}): ValidationResult;
/**
 * Enhanced validation error formatting with detailed context
 *
 * @param messages - Array of validation messages
 * @param options - Formatting options
 * @returns Formatted error message with context
 */
export declare function formatValidationErrorsDetailed(messages: ValidationMessage[], options?: {
    includeContext?: boolean;
    includeErrorCodes?: boolean;
    includeFilePaths?: boolean;
    maxContextLength?: number;
}): string;
/**
 * Convert validation errors to a user-friendly error message (legacy format)
 *
 * @param messages - Array of validation messages
 * @returns Formatted error message
 */
export declare function formatValidationErrors(messages: ValidationMessage[]): string;
/**
 * Generate a comprehensive validation report with recommendations
 *
 * @param result - Validation result to analyze
 * @param options - Report generation options
 * @returns Detailed validation report with actionable insights
 */
export declare function generateValidationReport(result: ValidationResult, options?: {
    includeSuccessSummary?: boolean;
    includeSuggestions?: boolean;
    includeDetailedContext?: boolean;
    groupByFile?: boolean;
}): string;
//# sourceMappingURL=validation.d.ts.map