/**
 * Type definitions for Bundle Package File Format
 */

export type {
  BundleVersion,
  EntrypointMap,
  BundleFile,
  BundleManifest,
  BundleInfo,
  CreateBundleOptions,
  AddFileOptions,
  SerializationOptions,
  ParseOptions,
} from './bundle.js';

export {
  BundleError,
  BundleParseError,
  BundleValidationError,
  FileNotFoundError,
  EntrypointNotFoundError,
  ZipOperationError,
  BundleSizeError,
  UnsupportedVersionError,
  SchemaValidationError,
} from './errors.js';

export {
  ValidationSeverity,
  ValidationRules,
  ValidationResultBuilder,
} from './validation.js';

export type {
  ValidationMessage,
  ValidationResult as DetailedValidationResult,
  ValidationOptions,
  ValidationRule,
  ValidationContext,
} from './validation.js';
