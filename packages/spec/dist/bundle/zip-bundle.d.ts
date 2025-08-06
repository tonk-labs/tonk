import { default as JSZip } from 'jszip';
import { Bundle } from './bundle.js';
import { BundleManifestType } from '../schemas/bundle.js';
import { BundleManifest, BundleFile, BundleInfo, AddFileOptions, ParseOptions, SerializationOptions } from '../types/bundle.js';
import { ValidationOptions, ValidationResult } from '../types/validation.js';

/**
 * ZIP-based implementation of the Bundle interface
 */
export declare class ZipBundle extends Bundle {
    private zip;
    private _manifest;
    private _sourceData;
    /**
     * Create a new ZipBundle instance
     * @param zip - JSZip instance containing the bundle data
     * @param manifest - Validated bundle manifest
     * @param sourceData - Original source data (optional)
     */
    constructor(zip: JSZip, manifest: BundleManifestType, sourceData?: ArrayBuffer);
    get manifest(): BundleManifest;
    get data(): ArrayBuffer | null;
    getFile(path: string): BundleFile | null;
    getFileData(path: string): Promise<ArrayBuffer | null>;
    hasFile(path: string): boolean;
    listFiles(): BundleFile[];
    getFileCount(): number;
    getEntrypoint(name: string): string | null;
    hasEntrypoint(name: string): boolean;
    listEntrypoints(): Record<string, string>;
    getEntrypointNames(): string[];
    addFile(file: Omit<BundleFile, 'length'>, data: ArrayBuffer, options?: AddFileOptions): Promise<void>;
    updateFile(path: string, data: ArrayBuffer, contentType?: string): Promise<void>;
    removeFile(path: string): Promise<void>;
    setEntrypoint(name: string, path: string): void;
    removeEntrypoint(name: string): void;
    validate(options?: ValidationOptions): ValidationResult;
    isValid(options?: ValidationOptions): boolean;
    isFileCompressed(path: string): boolean;
    getUncompressedSize(path: string): number | null;
    getBundleInfo(): BundleInfo;
    estimateBundleSize(): number;
    clone(): Promise<ZipBundle>;
    merge(other: Bundle, options?: {
        conflictResolution?: 'error' | 'skip' | 'replace';
        entrypointConflictResolution?: 'error' | 'skip' | 'replace';
    }): Promise<ZipBundle>;
    toArrayBuffer(options?: SerializationOptions): Promise<ArrayBuffer>;
    private updateManifestInZip;
    static createEmpty(options?: {
        version?: number;
    }): Promise<ZipBundle>;
    static fromFiles(files: Map<string, ArrayBuffer>, options?: {
        contentTypes?: Map<string, string>;
    }): Promise<ZipBundle>;
    static parse(data: ArrayBuffer, options?: ParseOptions): Promise<ZipBundle>;
}
//# sourceMappingURL=zip-bundle.d.ts.map