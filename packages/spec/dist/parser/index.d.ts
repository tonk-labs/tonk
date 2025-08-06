/**
 * Bundle Parser Module
 *
 * This module implements Phase 3 of the Bundle Package File Format implementation:
 * ZIP-based bundle parsing with comprehensive validation.
 *
 * Key features:
 * - JSZip-based ZIP archive parsing
 * - Zod schema validation for manifest.json
 * - File reference validation between manifest and ZIP contents
 * - Comprehensive error handling and reporting
 * - Support for both strict and lenient validation modes
 */
export { parseBundle, extractManifest, buildFileMap, validateFileReferences, createBundleFromZip, isValidZip, } from './zip-parser.js';
export { validateManifestData, validateZipManifestConsistency, validateBundleSize, validateBundle, validateBundleComprehensive, detectCircularEntrypointReferences, formatValidationErrors, formatValidationErrorsDetailed, generateValidationReport, } from './validation.js';
export type { ParseOptions } from '../types/bundle.js';
export type { ValidationResult, ValidationMessage, } from '../types/validation.js';
export { BundleParseError, BundleValidationError } from '../types/errors.js';
//# sourceMappingURL=index.d.ts.map