import type { Manifest } from '@tonk/core/slim';
import { Bundle, initializeTonk } from '@tonk/core/slim';
import { bundleStorage } from './bundleStorage';
import type { Bundle as BundleType } from '../types';

export class BundleManager {
  private initPromise: Promise<void> | null = null;

  private async ensureInitialized() {
    if (!this.initPromise) {
      // Explicitly load WASM from the root (where we copied it)
      this.initPromise = initializeTonk({ wasmPath: '/tonk_core_bg.wasm' });
    }
    await this.initPromise;
  }

  /**
   * Load a bundle from a File object, parse it, validate it, and save it to storage.
   * @param file The bundle file (.tonk)
   * @returns The bundle ID
   */
  async loadBundleFromFile(file: File): Promise<string> {
    await this.ensureInitialized();

    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);

    // Parse bundle to validate and get manifest
    let bundle: Bundle;
    try {
      bundle = await Bundle.fromBytes(bytes);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      throw new Error(`Invalid bundle: ${message}`);
    }

    let manifest: Manifest;
    try {
      manifest = await bundle.getManifest();
    } finally {
      bundle.free();
    }

    if (!manifest.entrypoints || manifest.entrypoints.length === 0) {
      throw new Error('Bundle has no entrypoints');
    }

    const id = crypto.randomUUID();

    // Save bundle with full cached manifest to skip redundant Bundle.fromBytes in SW
    await bundleStorage.save(id, {
      name: file.name,
      size: file.size,
      bytes,
      manifest, // Full Manifest object
    });

    return id;
  }

  /**
   * Delete a bundle by ID.
   * @param id Bundle ID
   */
  async deleteBundle(id: string): Promise<void> {
    await bundleStorage.delete(id);
  }

  /**
   * List all bundles.
   * @returns Array of bundles (metadata only)
   */
  async listBundles(): Promise<BundleType[]> {
    return bundleStorage.list();
  }
}

export const bundleManager = new BundleManager();
