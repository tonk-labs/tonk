import { z } from 'zod';

/**
 * Version schema - must be a positive integer
 */
export declare const BundleVersionSchema: z.ZodNumber;
/**
 * MIME type validation schema
 * Validates common MIME types with proper format
 */
export declare const MimeTypeSchema: z.ZodString;
/**
 * Virtual file path validation schema
 * Must start with / and contain valid path characters
 */
export declare const VirtualPathSchema: z.ZodString;
/**
 * ISO 8601 timestamp validation schema
 */
export declare const ISO8601Schema: z.ZodString;
/**
 * Bundle file metadata schema
 */
export declare const BundleFileSchema: z.ZodObject<{
    path: z.ZodString;
    length: z.ZodNumber;
    contentType: z.ZodString;
    compressed: z.ZodOptional<z.ZodBoolean>;
    uncompressedSize: z.ZodOptional<z.ZodNumber>;
    lastModified: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
/**
 * Entrypoint map schema
 * Keys are entrypoint names, values are file paths
 */
export declare const EntrypointMapSchema: z.ZodRecord<z.ZodString, z.ZodString>;
/**
 * Bundle manifest schema
 */
export declare const BundleManifestSchema: z.ZodObject<{
    version: z.ZodNumber;
    name: z.ZodOptional<z.ZodString>;
    description: z.ZodOptional<z.ZodString>;
    createdAt: z.ZodOptional<z.ZodString>;
    entrypoints: z.ZodRecord<z.ZodString, z.ZodString>;
    files: z.ZodArray<z.ZodObject<{
        path: z.ZodString;
        length: z.ZodNumber;
        contentType: z.ZodString;
        compressed: z.ZodOptional<z.ZodBoolean>;
        uncompressedSize: z.ZodOptional<z.ZodNumber>;
        lastModified: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    metadata: z.ZodOptional<z.ZodRecord<z.ZodString, z.ZodUnknown>>;
}, z.core.$strip>;
/**
 * Bundle validation options schema
 */
export declare const ValidationOptionsSchema: z.ZodObject<{
    includeWarnings: z.ZodDefault<z.ZodBoolean>;
    includeInfo: z.ZodDefault<z.ZodBoolean>;
    failFast: z.ZodDefault<z.ZodBoolean>;
    maxBundleSize: z.ZodOptional<z.ZodNumber>;
    maxFileCount: z.ZodOptional<z.ZodNumber>;
    strictMimeTypes: z.ZodDefault<z.ZodBoolean>;
    customRules: z.ZodDefault<z.ZodArray<z.ZodAny>>;
}, z.core.$strip>;
/**
 * Parse options schema for bundle parsing
 */
export declare const ParseOptionsSchema: z.ZodObject<{
    strictValidation: z.ZodDefault<z.ZodBoolean>;
    validateFileReferences: z.ZodDefault<z.ZodBoolean>;
    maxSize: z.ZodOptional<z.ZodNumber>;
}, z.core.$strip>;
/**
 * Serialization options schema
 */
export declare const SerializationOptionsSchema: z.ZodObject<{
    compressionLevel: z.ZodDefault<z.ZodNumber>;
    useZip64: z.ZodDefault<z.ZodBoolean>;
    comment: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
/**
 * Create bundle options schema
 */
export declare const CreateBundleOptionsSchema: z.ZodObject<{
    version: z.ZodDefault<z.ZodNumber>;
    name: z.ZodOptional<z.ZodString>;
    description: z.ZodOptional<z.ZodString>;
    metadata: z.ZodOptional<z.ZodRecord<z.ZodString, z.ZodUnknown>>;
}, z.core.$strip>;
/**
 * Add file options schema
 */
export declare const AddFileOptionsSchema: z.ZodObject<{
    contentType: z.ZodOptional<z.ZodString>;
    compress: z.ZodDefault<z.ZodBoolean>;
    replace: z.ZodDefault<z.ZodBoolean>;
    lastModified: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export type BundleVersionType = z.infer<typeof BundleVersionSchema>;
export type MimeTypeType = z.infer<typeof MimeTypeSchema>;
export type VirtualPathType = z.infer<typeof VirtualPathSchema>;
export type BundleFileType = z.infer<typeof BundleFileSchema>;
export type EntrypointMapType = z.infer<typeof EntrypointMapSchema>;
export type BundleManifestType = z.infer<typeof BundleManifestSchema>;
export type ValidationOptionsType = z.infer<typeof ValidationOptionsSchema>;
export type ParseOptionsType = z.infer<typeof ParseOptionsSchema>;
export type SerializationOptionsType = z.infer<typeof SerializationOptionsSchema>;
export type CreateBundleOptionsType = z.infer<typeof CreateBundleOptionsSchema>;
export type AddFileOptionsType = z.infer<typeof AddFileOptionsSchema>;
//# sourceMappingURL=bundle.d.ts.map