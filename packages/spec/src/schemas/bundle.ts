/**
 * Zod validation schemas for Bundle Package File Format
 * Using Zod 4 for runtime validation and TypeScript integration
 */

import { z } from 'zod';

/**
 * Version schema - must be a positive integer
 */
export const BundleVersionSchema = z
  .number()
  .int()
  .positive()
  .describe('Bundle format version');

/**
 * MIME type validation schema
 * Validates common MIME types with proper format
 */
export const MimeTypeSchema = z
  .string()
  .regex(
    /^[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_]*\/[a-zA-Z0-9][a-zA-Z0-9!#$&\-^_.]*$/,
    'Invalid MIME type format'
  )
  .describe('MIME type for file content');

/**
 * Virtual file path validation schema
 * Must start with / and contain valid path characters
 */
export const VirtualPathSchema = z
  .string()
  .min(1)
  .regex(/^\//, 'Path must start with /')
  .regex(/^\/[a-zA-Z0-9._\-/]*$/, 'Path contains invalid characters')
  .refine(path => !path.includes('//'), 'Path cannot contain double slashes')
  .refine(path => !path.includes('/./'), 'Path cannot contain /./')
  .refine(path => !path.includes('/../'), 'Path cannot contain /../')
  .describe('Virtual file path within bundle');

/**
 * ISO 8601 timestamp validation schema
 */
export const ISO8601Schema = z
  .string()
  .regex(
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?Z?$/,
    'Must be a valid ISO 8601 timestamp'
  )
  .describe('ISO 8601 timestamp');

/**
 * Bundle file metadata schema
 */
export const BundleFileSchema = z.object({
  path: VirtualPathSchema,
  length: z.number().int().min(0).describe('Size of file data in bytes'),
  contentType: MimeTypeSchema,
  compressed: z
    .boolean()
    .optional()
    .describe('Whether file data is compressed in ZIP'),
  uncompressedSize: z
    .number()
    .int()
    .min(0)
    .optional()
    .describe('Original uncompressed size'),
  lastModified: ISO8601Schema.optional(),
});

/**
 * Entrypoint map schema
 * Keys are entrypoint names, values are file paths
 */
export const EntrypointMapSchema = z
  .record(z.string().min(1), VirtualPathSchema)
  .describe('Mapping of entrypoint names to file paths');

/**
 * Bundle manifest schema
 */
export const BundleManifestSchema = z.object({
  version: BundleVersionSchema,
  name: z.string().min(1).optional().describe('Human-readable bundle name'),
  description: z.string().optional().describe('Bundle description'),
  createdAt: ISO8601Schema.optional().describe('Bundle creation timestamp'),
  entrypoints: EntrypointMapSchema,
  files: z.array(BundleFileSchema).describe('Array of all files in bundle'),
  metadata: z
    .record(z.string(), z.unknown())
    .optional()
    .describe('Optional extensibility metadata'),
});

/**
 * Bundle validation options schema
 */
export const ValidationOptionsSchema = z.object({
  includeWarnings: z.boolean().default(true),
  includeInfo: z.boolean().default(false),
  failFast: z.boolean().default(false),
  maxBundleSize: z.number().int().min(1).optional(),
  maxFileCount: z.number().int().min(1).optional(),
  strictMimeTypes: z.boolean().default(true),
  customRules: z.array(z.any()).default([]), // Custom rules will be typed separately
});

/**
 * Parse options schema for bundle parsing
 */
export const ParseOptionsSchema = z.object({
  strictValidation: z.boolean().default(true),
  validateFileReferences: z.boolean().default(true),
  maxSize: z.number().int().min(1).optional(),
});

/**
 * Serialization options schema
 */
export const SerializationOptionsSchema = z.object({
  compressionLevel: z.number().int().min(0).max(9).default(6),
  useZip64: z.boolean().default(false),
  comment: z.string().optional(),
});

/**
 * Create bundle options schema
 */
export const CreateBundleOptionsSchema = z.object({
  version: BundleVersionSchema.default(1),
  name: z.string().min(1).optional(),
  description: z.string().optional(),
  metadata: z.record(z.string(), z.unknown()).optional(),
});

/**
 * Add file options schema
 */
export const AddFileOptionsSchema = z.object({
  contentType: MimeTypeSchema.optional(),
  compress: z.boolean().default(true),
  replace: z.boolean().default(false),
  lastModified: ISO8601Schema.optional(),
});

// Export inferred types for TypeScript integration
export type BundleVersionType = z.infer<typeof BundleVersionSchema>;
export type MimeTypeType = z.infer<typeof MimeTypeSchema>;
export type VirtualPathType = z.infer<typeof VirtualPathSchema>;
export type BundleFileType = z.infer<typeof BundleFileSchema>;
export type EntrypointMapType = z.infer<typeof EntrypointMapSchema>;
export type BundleManifestType = z.infer<typeof BundleManifestSchema>;
export type ValidationOptionsType = z.infer<typeof ValidationOptionsSchema>;
export type ParseOptionsType = z.infer<typeof ParseOptionsSchema>;
export type SerializationOptionsType = z.infer<
  typeof SerializationOptionsSchema
>;
export type CreateBundleOptionsType = z.infer<typeof CreateBundleOptionsSchema>;
export type AddFileOptionsType = z.infer<typeof AddFileOptionsSchema>;
