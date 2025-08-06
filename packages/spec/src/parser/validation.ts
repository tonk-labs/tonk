/**
 * Parser Validation Utilities
 *
 * This module provides schema validation utilities for the ZIP-based bundle parser.
 * It complements the Zod schemas with parser-specific validation logic.
 */

import {
  BundleManifestSchema,
  type BundleManifestType,
} from '../schemas/bundle.js';
import type {
  ValidationResult,
  ValidationMessage,
  ValidationOptions,
} from '../types/validation.js';
import {
  ValidationResultBuilder,
  ValidationSeverity,
} from '../types/validation.js';
import JSZip from 'jszip';

/**
 * Detect circular references in entrypoint definitions
 *
 * This function performs graph analysis to detect cycles where entrypoints
 * reference each other in a circular fashion.
 *
 * @param entrypoints - Record of entrypoint names to file paths
 * @returns Array of detected circular reference chains
 */
export function detectCircularEntrypointReferences(
  entrypoints: Record<string, string>
): string[] {
  const graph = new Map<string, Set<string>>();
  const entrypointToFile = new Map<string, string>();
  const fileToEntrypoints = new Map<string, string[]>();

  // Build the graph relationships
  for (const [name, filePath] of Object.entries(entrypoints)) {
    entrypointToFile.set(name, filePath);

    if (!fileToEntrypoints.has(filePath)) {
      fileToEntrypoints.set(filePath, []);
    }
    fileToEntrypoints.get(filePath)!.push(name);

    if (!graph.has(name)) {
      graph.set(name, new Set());
    }
  }

  // Add edges based on file relationships
  for (const [entrypoint, filePath] of entrypointToFile) {
    const otherEntrypoints = fileToEntrypoints.get(filePath) || [];
    for (const otherEntrypoint of otherEntrypoints) {
      if (otherEntrypoint !== entrypoint) {
        graph.get(entrypoint)!.add(otherEntrypoint);
      }
    }
  }

  // Detect cycles using DFS
  const visited = new Set<string>();
  const recursionStack = new Set<string>();
  const cycles: string[] = [];

  function dfs(node: string, path: string[]): void {
    if (recursionStack.has(node)) {
      // Found a cycle
      const cycleStart = path.indexOf(node);
      if (cycleStart >= 0) {
        const cycle = path.slice(cycleStart).concat([node]);
        cycles.push(cycle.join(' -> '));
      }
      return;
    }

    if (visited.has(node)) {
      return;
    }

    visited.add(node);
    recursionStack.add(node);
    path.push(node);

    const neighbors = graph.get(node) || new Set();
    for (const neighbor of neighbors) {
      dfs(neighbor, [...path]);
    }

    recursionStack.delete(node);
    path.pop();
  }

  for (const entrypoint of Object.keys(entrypoints)) {
    if (!visited.has(entrypoint)) {
      dfs(entrypoint, []);
    }
  }

  return cycles;
}

/**
 * Validate a parsed manifest object using Zod schemas
 *
 * @param manifestData - Raw manifest data to validate
 * @param strict - Whether to use strict validation mode
 * @returns ValidationResult with success/error information
 */
export function validateManifestData(
  manifestData: unknown,
  strict = true
): ValidationResult {
  const builder = new ValidationResultBuilder();

  try {
    BundleManifestSchema.parse(manifestData);
    return builder.build(); // No errors, returns valid result
  } catch (error) {
    if (strict) {
      if (error && typeof error === 'object' && 'issues' in error) {
        // Handle Zod validation errors
        const zodError = error as {
          issues: Array<{ path: (string | number)[]; message: string }>;
        };
        for (const issue of zodError.issues) {
          builder.addError(issue.message, 'VALIDATION_ERROR', {
            field: issue.path.join('.'),
          });
        }
      } else {
        builder.addError(
          error instanceof Error ? error.message : 'Unknown validation error',
          'VALIDATION_ERROR',
          { field: 'manifest' }
        );
      }
    } else {
      // In non-strict mode, add a warning but still return valid
      builder.addWarning(
        'Validation skipped in non-strict mode',
        'VALIDATION_WARNING',
        { field: 'manifest' }
      );
    }

    return builder.build();
  }
}

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
export function validateZipManifestConsistency(
  zip: JSZip,
  manifest: BundleManifestType
): ValidationResult {
  const builder = new ValidationResultBuilder();

  // Check that all manifest files exist in ZIP
  for (const manifestFile of manifest.files) {
    const zipPath = manifestFile.path.startsWith('/')
      ? manifestFile.path.slice(1)
      : manifestFile.path;

    const zipFile = zip.file(zipPath);
    if (!zipFile) {
      builder.addError(
        `File declared in manifest not found in ZIP archive: ${manifestFile.path}`,
        'FILE_NOT_FOUND',
        { filePath: manifestFile.path }
      );
    } else {
      // Validate file size consistency if available
      if (manifestFile.length !== undefined) {
        // Note: We can't easily get uncompressed size from JSZip without reading the file
        // This would be an async operation, so we skip this validation for now
        // In a real implementation, you might want to make this function async
      }
    }
  }

  // Check that all entrypoints reference existing files
  if (manifest.entrypoints) {
    for (const [entrypointName, filePath] of Object.entries(
      manifest.entrypoints
    )) {
      const fileExists = manifest.files.some(file => file.path === filePath);
      if (!fileExists) {
        builder.addError(
          `Entrypoint '${entrypointName}' references file not declared in manifest: ${filePath}`,
          'ENTRYPOINT_INVALID',
          { entrypointName, filePath }
        );
      }
    }
  }

  // Check for circular entrypoint references with improved detection
  if (manifest.entrypoints) {
    const circularRefs = detectCircularEntrypointReferences(
      manifest.entrypoints
    );

    if (circularRefs.length > 0) {
      builder.addError(
        `Circular entrypoint references detected: ${circularRefs.join(', ')}`,
        'CIRCULAR_ENTRYPOINT_REFERENCE',
        { circularRefs, count: circularRefs.length }
      );
    }
  }

  return builder.build();
}

/**
 * Validate bundle size constraints
 *
 * @param zipSize - Size of the ZIP archive in bytes
 * @param manifest - Bundle manifest
 * @param maxSize - Maximum allowed size in bytes (optional)
 * @returns ValidationResult with size validation
 */
export function validateBundleSize(
  zipSize: number,
  manifest: BundleManifestType,
  maxSize?: number
): ValidationResult {
  const builder = new ValidationResultBuilder();

  // Check maximum size constraint
  if (maxSize && zipSize > maxSize) {
    builder.addError(
      `Bundle size ${zipSize} bytes exceeds maximum allowed size ${maxSize} bytes`,
      'SIZE_EXCEEDED',
      { zipSize, maxSize }
    );
  }

  // Check for reasonable size constraints (e.g., warn if very large)
  const warningSize = 100 * 1024 * 1024; // 100MB
  if (zipSize > warningSize) {
    builder.addWarning(
      `Bundle size ${zipSize} bytes is unusually large (>100MB)`,
      'SIZE_WARNING',
      { zipSize, warningSize }
    );
  }

  // Validate that declared file sizes are reasonable
  let declaredTotalSize = 0;
  for (const file of manifest.files) {
    if (file.length !== undefined) {
      declaredTotalSize += file.length;
    }
  }

  // Check if compressed size is reasonable compared to declared sizes
  if (declaredTotalSize > 0 && zipSize > declaredTotalSize * 2) {
    builder.addWarning(
      `ZIP size ${zipSize} is unusually large compared to declared file sizes ${declaredTotalSize}`,
      'COMPRESSION_WARNING',
      { zipSize, declaredTotalSize }
    );
  }

  return builder.build();
}

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
export function validateBundleComprehensive(
  zip: JSZip,
  manifest: BundleManifestType,
  zipSize: number,
  options: ValidationOptions = {}
): ValidationResult {
  const {
    maxBundleSize,
    maxFileCount,
    strictMimeTypes = false,
    includeWarnings = true,
    includeInfo = false,
    failFast = false,
    customRules = [],
  } = options;

  const builder = new ValidationResultBuilder();

  // Track validation context for error reporting
  const validationContext = {
    bundleSize: zipSize,
    fileCount: manifest.files.length,
    entrypointCount: Object.keys(manifest.entrypoints || {}).length,
    timestamp: new Date().toISOString(),
  };

  try {
    // 1. Validate manifest structure with enhanced error context
    const manifestValidation = validateManifestData(manifest, true);
    manifestValidation.messages.forEach(msg => {
      const enhancedContext = { ...msg.context, ...validationContext };

      if (msg.severity === ValidationSeverity.ERROR) {
        builder.addError(msg.message, msg.code, enhancedContext, msg.filePath);
        if (failFast) return builder.build();
      } else if (
        msg.severity === ValidationSeverity.WARNING &&
        includeWarnings
      ) {
        builder.addWarning(
          msg.message,
          msg.code,
          enhancedContext,
          msg.filePath
        );
      } else if (msg.severity === ValidationSeverity.INFO && includeInfo) {
        builder.addInfo(msg.message, msg.code, enhancedContext, msg.filePath);
      }
    });

    // 2. Validate ZIP/manifest consistency
    const consistencyValidation = validateZipManifestConsistency(zip, manifest);
    consistencyValidation.messages.forEach(msg => {
      const enhancedContext = { ...msg.context, ...validationContext };

      if (msg.severity === ValidationSeverity.ERROR) {
        builder.addError(msg.message, msg.code, enhancedContext, msg.filePath);
        if (failFast) return builder.build();
      } else if (
        msg.severity === ValidationSeverity.WARNING &&
        includeWarnings
      ) {
        builder.addWarning(
          msg.message,
          msg.code,
          enhancedContext,
          msg.filePath
        );
      } else if (msg.severity === ValidationSeverity.INFO && includeInfo) {
        builder.addInfo(msg.message, msg.code, enhancedContext, msg.filePath);
      }
    });

    // 3. Validate size constraints with custom limits
    const sizeValidation = validateBundleSize(zipSize, manifest, maxBundleSize);
    sizeValidation.messages.forEach(msg => {
      const enhancedContext = { ...msg.context, ...validationContext };

      if (msg.severity === ValidationSeverity.ERROR) {
        builder.addError(msg.message, msg.code, enhancedContext, msg.filePath);
        if (failFast) return builder.build();
      } else if (
        msg.severity === ValidationSeverity.WARNING &&
        includeWarnings
      ) {
        builder.addWarning(
          msg.message,
          msg.code,
          enhancedContext,
          msg.filePath
        );
      } else if (msg.severity === ValidationSeverity.INFO && includeInfo) {
        builder.addInfo(msg.message, msg.code, enhancedContext, msg.filePath);
      }
    });

    // 4. Validate file count limits
    if (maxFileCount && manifest.files.length > maxFileCount) {
      builder.addError(
        `Bundle contains ${manifest.files.length} files, which exceeds the maximum allowed ${maxFileCount}`,
        'FILE_COUNT_EXCEEDED',
        {
          actualCount: manifest.files.length,
          maxCount: maxFileCount,
          ...validationContext,
        }
      );
      if (failFast) return builder.build();
    }

    // 5. Validate MIME types if strict mode is enabled
    if (strictMimeTypes) {
      const mimeTypeRegex =
        /^[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_]*\/[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.]*$/;
      for (const file of manifest.files) {
        if (file.contentType && !mimeTypeRegex.test(file.contentType)) {
          builder.addError(
            `Invalid MIME type for file ${file.path}: "${file.contentType}"`,
            'INVALID_MIME_TYPE',
            {
              filePath: file.path,
              mimeType: file.contentType,
              ...validationContext,
            }
          );
          if (failFast) return builder.build();
        }
      }
    }

    // 6. Apply custom validation rules
    for (const rule of customRules) {
      try {
        const ruleMessages = rule.validate({
          manifest,
          zipData: undefined, // ZIP data not exposed in this context for security
          options,
          createMessage: (severity, message, code, context) => ({
            severity,
            message,
            code,
            context: { ...context, rule: rule.id, ...validationContext },
          }),
        });

        ruleMessages.forEach(msg => {
          if (msg.severity === ValidationSeverity.ERROR) {
            builder.addError(msg.message, msg.code, msg.context);
            if (failFast) return builder.build();
          } else if (
            msg.severity === ValidationSeverity.WARNING &&
            includeWarnings
          ) {
            builder.addWarning(msg.message, msg.code, msg.context);
          } else if (msg.severity === ValidationSeverity.INFO && includeInfo) {
            builder.addInfo(msg.message, msg.code, msg.context);
          }
        });
      } catch (ruleError) {
        builder.addError(
          `Custom validation rule "${rule.id}" failed: ${ruleError instanceof Error ? ruleError.message : 'Unknown error'}`,
          'CUSTOM_RULE_ERROR',
          {
            ruleId: rule.id,
            error:
              ruleError instanceof Error
                ? ruleError.message
                : String(ruleError),
            ...validationContext,
          }
        );
        if (failFast) return builder.build();
      }
    }

    // 7. Add summary information if requested
    if (includeInfo) {
      builder.addInfo(
        `Bundle validation completed: ${manifest.files.length} files, ${Object.keys(manifest.entrypoints || {}).length} entrypoints`,
        'VALIDATION_SUMMARY',
        validationContext
      );
    }
  } catch (error) {
    // Preserve stack trace and add comprehensive error context
    const errorMessage =
      error instanceof Error ? error.message : 'Unknown validation error';
    const errorStack = error instanceof Error ? error.stack : undefined;

    builder.addError(
      `Validation process failed: ${errorMessage}`,
      'VALIDATION_PROCESS_ERROR',
      {
        error: errorMessage,
        stack: errorStack,
        ...validationContext,
      }
    );
  }

  return builder.build();
}

/**
 * Perform bundle validation (simplified version for backward compatibility)
 *
 * @param zip - JSZip instance containing bundle data
 * @param manifest - Bundle manifest
 * @param zipSize - Size of the ZIP archive
 * @param options - Basic validation options
 * @returns Combined validation result
 */
export function validateBundle(
  zip: JSZip,
  manifest: BundleManifestType,
  zipSize: number,
  options: {
    maxSize?: number;
    strictValidation?: boolean;
    validateFileReferences?: boolean;
  } = {}
): ValidationResult {
  const { maxSize } = options;

  // Use the comprehensive validation with mapped options
  const validationOptions: ValidationOptions = {
    maxBundleSize: maxSize,
    includeWarnings: true,
    includeInfo: false,
    failFast: false,
  };

  return validateBundleComprehensive(zip, manifest, zipSize, validationOptions);
}

/**
 * Enhanced validation error formatting with detailed context
 *
 * @param messages - Array of validation messages
 * @param options - Formatting options
 * @returns Formatted error message with context
 */
export function formatValidationErrorsDetailed(
  messages: ValidationMessage[],
  options: {
    includeContext?: boolean;
    includeErrorCodes?: boolean;
    includeFilePaths?: boolean;
    maxContextLength?: number;
  } = {}
): string {
  const {
    includeContext = true,
    includeErrorCodes = true,
    includeFilePaths = true,
    maxContextLength = 200,
  } = options;

  if (messages.length === 0) {
    return 'No validation messages';
  }

  const groupedMessages = new Map<string, ValidationMessage[]>();

  for (const message of messages) {
    const category = message.code.split('_')[0];
    if (!groupedMessages.has(category)) {
      groupedMessages.set(category, []);
    }
    groupedMessages.get(category)!.push(message);
  }

  const lines: string[] = [];
  for (const [category, categoryMessages] of groupedMessages) {
    lines.push(
      `\n${category.toUpperCase()} (${categoryMessages.length} ${categoryMessages.length === 1 ? 'issue' : 'issues'}):`
    );

    for (const message of categoryMessages) {
      const prefix =
        message.severity === ValidationSeverity.ERROR
          ? '❌'
          : message.severity === ValidationSeverity.WARNING
            ? '⚠️'
            : 'ℹ️';

      let line = `  ${prefix} ${message.message}`;

      if (includeErrorCodes) {
        line += ` [${message.code}]`;
      }

      if (includeFilePaths && message.filePath) {
        line += ` (${message.filePath})`;
      }

      lines.push(line);

      if (
        includeContext &&
        message.context &&
        Object.keys(message.context).length > 0
      ) {
        const contextString = JSON.stringify(message.context, null, 2);
        const truncatedContext =
          contextString.length > maxContextLength
            ? contextString.substring(0, maxContextLength) + '...'
            : contextString;
        lines.push(`    Context: ${truncatedContext}`);
      }

      if (message.suggestion) {
        lines.push(`    💡 Suggestion: ${message.suggestion}`);
      }
    }
  }

  return lines.join('\n');
}

/**
 * Convert validation errors to a user-friendly error message (legacy format)
 *
 * @param messages - Array of validation messages
 * @returns Formatted error message
 */
export function formatValidationErrors(messages: ValidationMessage[]): string {
  if (messages.length === 0) {
    return 'No validation errors';
  }

  const groupedMessages = new Map<string, ValidationMessage[]>();

  for (const message of messages) {
    const category = message.code.split('_')[0];
    if (!groupedMessages.has(category)) {
      groupedMessages.set(category, []);
    }
    groupedMessages.get(category)!.push(message);
  }

  const lines: string[] = [];
  for (const [category, categoryMessages] of groupedMessages) {
    lines.push(`${category}:`);
    for (const message of categoryMessages) {
      const prefix =
        message.severity === ValidationSeverity.ERROR
          ? '❌'
          : message.severity === ValidationSeverity.WARNING
            ? '⚠️'
            : 'ℹ️';
      lines.push(`  ${prefix} ${message.message}`);
    }
  }

  return lines.join('\n');
}

/**
 * Generate a comprehensive validation report with recommendations
 *
 * @param result - Validation result to analyze
 * @param options - Report generation options
 * @returns Detailed validation report with actionable insights
 */
export function generateValidationReport(
  result: ValidationResult,
  options: {
    includeSuccessSummary?: boolean;
    includeSuggestions?: boolean;
    includeDetailedContext?: boolean;
    groupByFile?: boolean;
  } = {}
): string {
  const {
    includeSuccessSummary = true,
    includeSuggestions = true,
    includeDetailedContext = false,
    groupByFile = false,
  } = options;

  const lines: string[] = [];

  // Header
  lines.push('📋 Bundle Validation Report');
  lines.push('='.repeat(50));
  lines.push('');

  // Summary
  if (result.valid && includeSuccessSummary) {
    lines.push('✅ Validation Status: PASSED');
    lines.push(`📊 Total checks: ${result.messages.length}`);
    lines.push('');
  } else if (!result.valid) {
    lines.push('❌ Validation Status: FAILED');
    lines.push(
      `📊 Errors: ${result.errors.length}, Warnings: ${result.warnings.length}, Info: ${result.info.length}`
    );
    lines.push('');
  }

  // Group messages
  let groupedMessages: Map<string, ValidationMessage[]>;

  if (groupByFile) {
    groupedMessages = new Map();
    for (const message of result.messages) {
      const key = message.filePath || '(Bundle-level)';
      if (!groupedMessages.has(key)) {
        groupedMessages.set(key, []);
      }
      groupedMessages.get(key)!.push(message);
    }
  } else {
    groupedMessages = new Map();
    for (const message of result.messages) {
      const category = message.code.split('_')[0];
      if (!groupedMessages.has(category)) {
        groupedMessages.set(category, []);
      }
      groupedMessages.get(category)!.push(message);
    }
  }

  // Report details
  for (const [groupName, messages] of groupedMessages) {
    if (messages.length === 0) continue;

    lines.push(`📂 ${groupName.toUpperCase()}`);
    lines.push('-'.repeat(30));

    for (const message of messages) {
      const icon =
        message.severity === ValidationSeverity.ERROR
          ? '❌'
          : message.severity === ValidationSeverity.WARNING
            ? '⚠️'
            : 'ℹ️';

      lines.push(`${icon} ${message.message}`);

      if (message.filePath && !groupByFile) {
        lines.push(`   📁 File: ${message.filePath}`);
      }

      if (includeDetailedContext && message.context) {
        lines.push(
          `   🔍 Context: ${JSON.stringify(message.context, null, 4)}`
        );
      }

      if (includeSuggestions && message.suggestion) {
        lines.push(`   💡 Suggestion: ${message.suggestion}`);
      }

      lines.push('');
    }
  }

  // Footer recommendations
  if (includeSuggestions && !result.valid) {
    lines.push('🔧 Recommended Actions:');
    lines.push('-'.repeat(30));

    if (result.errors.length > 0) {
      lines.push('1. Fix all errors before proceeding with bundle operations');
    }

    if (result.warnings.length > 0) {
      lines.push('2. Review warnings to ensure optimal bundle configuration');
    }

    lines.push('3. Re-run validation after making changes');
    lines.push(
      '4. Consider using stricter validation settings for production bundles'
    );
  }

  return lines.join('\n');
}
