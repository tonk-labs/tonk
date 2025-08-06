/**
 * @tonk/spec - TypeScript specification library for Bundle Package File Format
 *
 * This library provides TypeScript types and utilities for parsing and manipulating
 * the Bundle Package File Format. It focuses on in-memory representation and manipulation.
 * Serialization and IO are handled by other libraries in the Tonk ecosystem.
 *
 * @packageDocumentation
 */
export type { BundleVersion, EntrypointMap, BundleFile, BundleManifest, BundleInfo, CreateBundleOptions, AddFileOptions, SerializationOptions, ParseOptions, } from './types/bundle.js';
export { BundleError, BundleParseError, BundleValidationError, FileNotFoundError, EntrypointNotFoundError, ZipOperationError, BundleSizeError, UnsupportedVersionError, SchemaValidationError, CircularReferenceError, ValidationContextError, EnhancedBundleError, } from './types/errors.js';
export { ValidationSeverity, ValidationRules, ValidationResultBuilder, } from './types/validation.js';
export type { ValidationMessage, ValidationResult as DetailedValidationResult, ValidationOptions, ValidationRule, ValidationContext, } from './types/validation.js';
export { Bundle } from './bundle/bundle.js';
export { ZipBundle } from './bundle/zip-bundle.js';
export { BundleVersionSchema, MimeTypeSchema, VirtualPathSchema, BundleFileSchema, EntrypointMapSchema, BundleManifestSchema, ValidationOptionsSchema, ParseOptionsSchema, SerializationOptionsSchema, CreateBundleOptionsSchema, AddFileOptionsSchema, } from './schemas/bundle.js';
export { validateManifest, validateBundleFile, validateEntrypoints, validateManifestZipConsistency, createValidationMessage, } from './schemas/validation.js';
export { parseBundle, extractManifest, buildFileMap, validateFileReferences, createBundleFromZip, isValidZip, validateManifestData, validateZipManifestConsistency, validateBundleSize, validateBundle, validateBundleComprehensive, detectCircularEntrypointReferences, formatValidationErrors, formatValidationErrorsDetailed, generateValidationReport, } from './parser/index.js';
export * from './types/index.js';
export * from './utils/index.js';
//# sourceMappingURL=index.d.ts.map