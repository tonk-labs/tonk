import { TonkCore, VirtualFileSystem } from "@tonk/core";

/**
 * Singleton class that holds references to tonk and vfs instances
 */
class TonkFileSystem {
  private static instance: TonkFileSystem;
  private _tonk: TonkCore | null = null;
  private _vfs: VirtualFileSystem | null = null; // TODO: Add proper VFS type when available

  private constructor() {}

  /**
   * Get the singleton instance
   */
  public static getInstance(): TonkFileSystem {
    if (!TonkFileSystem.instance) {
      TonkFileSystem.instance = new TonkFileSystem();
    }
    return TonkFileSystem.instance;
  }

  /**
   * Initialize the tonk instance from bytes
   * @param bytes - The bytes to initialize TonkCore from
   */
  public async initializeTonk(bytes: Uint8Array): Promise<void> {
    try {
      this._tonk = await TonkCore.fromBytes(bytes, { storage: { type: 'indexeddb' } });
      this._vfs = await this._tonk.getVfs();
      
    } catch (error) {
      console.error("Failed to initialize TonkCore from bytes:", error);
      throw error;
    }
  }

  /**
   * Get the tonk instance
   */
  public get tonk(): TonkCore | null {
    return this._tonk;
  }

  /**
   * Get the VFS instance
   */
  public get vfs(): VirtualFileSystem | null {
    return this._vfs;
  }

  /**
   * Check if both tonk and vfs are initialized
   */
  public get isInitialized(): boolean {
    return this._tonk !== null && this._vfs !== null;
  }

  /**
   * Reset the singleton (useful for testing)
   */
  public reset(): void {
    this._tonk = null;
    this._vfs = null;
  }
}

// Export the singleton instance
export const tonkFS = TonkFileSystem.getInstance();

// Export the class for testing purposes
export { TonkFileSystem };
