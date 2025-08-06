/**
 * Validation utilities for Bundle Package File Format
 * Integrates Zod validation with custom validation logic
 */

import { ZodError, ZodIssue } from 'zod';
import {
  ValidationResult,
  ValidationMessage,
  ValidationSeverity,
  ValidationResultBuilder,
  ValidationRules,
  ValidationOptions,
} from '../types/validation.js';
import {
  BundleManifestSchema,
  BundleFileSchema,
  EntrypointMapSchema,
  type BundleManifestType,
} from './bundle.js';

/**
 * Validates a bundle manifest using Zod schema
 */
export function validateManifest(
  manifest: unknown,
  options: Partial<ValidationOptions> = {}
): ValidationResult {
  const builder = new ValidationResultBuilder();

  try {
    // Validate with Zod schema
    const validatedManifest = BundleManifestSchema.parse(manifest);

    // Perform additional custom validations
    performCustomValidations(validatedManifest, builder, options);

    if (!builder.hasErrors()) {
      builder.addInfo(
        'Manifest validation passed',
        ValidationRules.ZOD_SCHEMA_VALIDATION
      );
    }
  } catch (error) {
    if (error instanceof ZodError) {
      addZodErrorsToBuilder(error, builder);
    } else {
      builder.addError(
        `Unexpected validation error: ${error instanceof Error ? error.message : 'Unknown error'}`,
        ValidationRules.ZOD_SCHEMA_VALIDATION
      );
    }
  }

  return builder.build();
}

/**
 * Validates individual bundle file metadata
 */
export function validateBundleFile(file: unknown): ValidationResult {
  const builder = new ValidationResultBuilder();

  try {
    BundleFileSchema.parse(file);
    builder.addInfo(
      'File metadata validation passed',
      ValidationRules.ZOD_SCHEMA_VALIDATION
    );
  } catch (error) {
    if (error instanceof ZodError) {
      addZodErrorsToBuilder(error, builder);
    } else {
      builder.addError(
        `Unexpected file validation error: ${error instanceof Error ? error.message : 'Unknown error'}`,
        ValidationRules.ZOD_SCHEMA_VALIDATION
      );
    }
  }

  return builder.build();
}

/**
 * Validates entrypoint mappings
 */
export function validateEntrypoints(entrypoints: unknown): ValidationResult {
  const builder = new ValidationResultBuilder();

  try {
    EntrypointMapSchema.parse(entrypoints);
    builder.addInfo(
      'Entrypoints validation passed',
      ValidationRules.ZOD_SCHEMA_VALIDATION
    );
  } catch (error) {
    if (error instanceof ZodError) {
      addZodErrorsToBuilder(error, builder);
    } else {
      builder.addError(
        `Unexpected entrypoints validation error: ${error instanceof Error ? error.message : 'Unknown error'}`,
        ValidationRules.ZOD_SCHEMA_VALIDATION
      );
    }
  }

  return builder.build();
}

/**
 * Performs custom validation logic beyond schema validation
 */
function performCustomValidations(
  manifest: BundleManifestType,
  builder: ValidationResultBuilder,
  options: Partial<ValidationOptions>
): void {
  // Validate unique file paths
  const filePaths = new Set<string>();
  const duplicatePaths: string[] = [];

  for (const file of manifest.files) {
    if (filePaths.has(file.path)) {
      duplicatePaths.push(file.path);
    } else {
      filePaths.add(file.path);
    }
  }

  if (duplicatePaths.length > 0) {
    builder.addError(
      `Duplicate file paths found: ${duplicatePaths.join(', ')}`,
      ValidationRules.UNIQUE_FILE_PATHS,
      { duplicatePaths }
    );
  }

  // Validate that entrypoints reference existing files
  const entrypointIssues: string[] = [];
  for (const [entrypointName, filePath] of Object.entries(
    manifest.entrypoints
  )) {
    if (!filePaths.has(filePath)) {
      entrypointIssues.push(`${entrypointName} -> ${filePath}`);
    }
  }

  if (entrypointIssues.length > 0) {
    builder.addError(
      `Entrypoints reference non-existent files: ${entrypointIssues.join(', ')}`,
      ValidationRules.VALID_ENTRYPOINTS,
      { invalidEntrypoints: entrypointIssues }
    );
  }

  // Check bundle size limits
  if (options.maxFileCount && manifest.files.length > options.maxFileCount) {
    builder.addError(
      `Bundle contains ${manifest.files.length} files, but maximum allowed is ${options.maxFileCount}`,
      ValidationRules.FILE_COUNT_LIMIT,
      { fileCount: manifest.files.length, maxFileCount: options.maxFileCount }
    );
  }

  // Check for circular entrypoint references
  const circularRefs = findCircularEntrypointReferences(manifest.entrypoints);
  if (circularRefs.length > 0) {
    builder.addWarning(
      `Potential circular entrypoint references detected: ${circularRefs.join(', ')}`,
      ValidationRules.VALID_ENTRYPOINTS,
      { circularReferences: circularRefs }
    );
  }

  // Validate version compatibility
  if (manifest.version < 1) {
    builder.addError(
      `Bundle version ${manifest.version} is not supported (minimum version is 1)`,
      ValidationRules.VALID_VERSION,
      { version: manifest.version, minimumVersion: 1 }
    );
  }

  if (manifest.version > 1) {
    builder.addWarning(
      `Bundle version ${manifest.version} is newer than expected (current version is 1)`,
      ValidationRules.VALID_VERSION,
      { version: manifest.version, currentVersion: 1 }
    );
  }
}

/**
 * Converts Zod validation errors to ValidationMessages
 */
function addZodErrorsToBuilder(
  zodError: ZodError,
  builder: ValidationResultBuilder
): void {
  for (const issue of zodError.issues) {
    const message = formatZodIssue(issue);
    const context = {
      path: issue.path,
      zodCode: issue.code,
      received: (issue as any).received, // Zod 4 compatibility
      expected: getExpectedValue(issue),
    };

    builder.addError(message, ValidationRules.ZOD_SCHEMA_VALIDATION, context);
  }
}

/**
 * Formats a Zod issue into a human-readable error message
 */
function formatZodIssue(issue: ZodIssue): string {
  const path =
    issue.path.length > 0 ? ` at path "${issue.path.join('.')}"` : '';

  switch (issue.code) {
    case 'invalid_type':
      return `Expected ${(issue as any).expected} but received ${(issue as any).received}${path}`;
    case 'too_small':
      return `Value${path} is too small: ${issue.message}`;
    case 'too_big':
      return `Value${path} is too big: ${issue.message}`;
    case 'custom':
      return `Validation failed${path}: ${issue.message}`;
    default:
      return `Validation error${path}: ${issue.message}`;
  }
}

/**
 * Extracts expected value information from Zod issue
 */
function getExpectedValue(issue: ZodIssue): unknown {
  switch (issue.code) {
    case 'invalid_type':
      return (issue as any).expected;
    case 'too_small':
      return `>= ${(issue as any).minimum}`;
    case 'too_big':
      return `<= ${(issue as any).maximum}`;
    default:
      return undefined;
  }
}

/**
 * Detects circular references in entrypoint mappings
 * Note: This is a basic implementation - more sophisticated cycle detection could be added
 */
function findCircularEntrypointReferences(
  entrypoints: Record<string, string>
): string[] {
  const circular: string[] = [];

  // Simple check for direct circular references (A -> B, B -> A)
  const pathToEntrypoint = new Map<string, string>();

  for (const [name, path] of Object.entries(entrypoints)) {
    const existingEntrypoint = pathToEntrypoint.get(path);
    if (
      existingEntrypoint &&
      entrypoints[existingEntrypoint] === entrypoints[name]
    ) {
      circular.push(`${name} <-> ${existingEntrypoint}`);
    }
    pathToEntrypoint.set(path, name);
  }

  return circular;
}

/**
 * Creates a validation message helper function
 */
export function createValidationMessage(
  severity: ValidationSeverity,
  message: string,
  code: string,
  context?: Record<string, unknown>,
  filePath?: string
): ValidationMessage {
  return {
    severity,
    message,
    code,
    context,
    filePath,
  };
}

/**
 * Validates that a manifest is consistent with ZIP contents
 */
export function validateManifestZipConsistency(
  manifest: BundleManifestType,
  zipFileNames: string[]
): ValidationResult {
  const builder = new ValidationResultBuilder();
  const zipFiles = new Set(
    zipFileNames.filter(name => name !== 'manifest.json')
  );
  const manifestFiles = new Set(manifest.files.map(f => f.path));

  // Check for files in manifest but not in ZIP
  const missingInZip = [...manifestFiles].filter(
    path => !zipFiles.has(path.slice(1))
  ); // Remove leading /
  if (missingInZip.length > 0) {
    builder.addError(
      `Files listed in manifest but missing from ZIP: ${missingInZip.join(', ')}`,
      ValidationRules.MANIFEST_ZIP_CONSISTENCY,
      { missingFiles: missingInZip }
    );
  }

  // Check for files in ZIP but not in manifest
  const missingInManifest = [...zipFiles].filter(
    path => !manifestFiles.has('/' + path)
  );
  if (missingInManifest.length > 0) {
    builder.addWarning(
      `Files in ZIP but not listed in manifest: ${missingInManifest.join(', ')}`,
      ValidationRules.MANIFEST_ZIP_CONSISTENCY,
      { extraFiles: missingInManifest }
    );
  }

  return builder.build();
}
