import type {
  Chunk,
  StorageAdapterInterface,
  StorageKey,
} from '@automerge/automerge-repo';
import { NodeFSStorageAdapter } from '@automerge/automerge-repo-storage-nodefs';
import { BundleStorageAdapter } from './bundleStorageAdapter.js';
import fs from 'fs';
import path from 'path';
import { rimraf } from 'rimraf';

export class CompositeStorageAdapter implements StorageAdapterInterface {
  #bundleStorage: BundleStorageAdapter;
  #fsStorage: NodeFSStorageAdapter;
  #baseDirectory: string;

  constructor(
    bundleStorage: BundleStorageAdapter,
    fsStorage: NodeFSStorageAdapter,
    baseDirectory: string
  ) {
    this.#bundleStorage = bundleStorage;
    this.#fsStorage = fsStorage;
    this.#baseDirectory = baseDirectory;
  }

  static async create(
    bundleBytes: Uint8Array,
    storageDir = 'automerge-repo-data'
  ): Promise<CompositeStorageAdapter> {
    const bundleStorage = await BundleStorageAdapter.fromBundle(bundleBytes);
    const fsStorage = new NodeFSStorageAdapter(storageDir);
    return new CompositeStorageAdapter(bundleStorage, fsStorage, storageDir);
  }

  async load(key: StorageKey): Promise<Uint8Array | undefined> {
    const fsData = await this.#fsStorage.load(key);
    if (fsData) return fsData;

    return await this.#bundleStorage.load(key);
  }

  async save(key: StorageKey, data: Uint8Array): Promise<void> {
    await this.#fsStorage.save(key, data);
  }

  private getFilePath(keyArray: string[]): string {
    const [firstKey, ...remainingKeys] = keyArray;
    return path.join(
      this.#baseDirectory,
      firstKey.slice(0, 2),
      firstKey.slice(2),
      ...remainingKeys
    );
  }

  async remove(key: StorageKey): Promise<void> {
    try {
      await this.#fsStorage.remove(key);
    } catch (error: any) {
      if (error.code === 'EPERM' || error.code === 'EISDIR') {
        const filePath = this.getFilePath(key);
        try {
          const stats = await fs.promises.stat(filePath);
          if (stats.isDirectory()) {
            await rimraf(filePath);
          } else {
            throw error;
          }
        } catch (statError: any) {
          if (statError.code !== 'ENOENT') {
            throw statError;
          }
        }
      } else if (error.code !== 'ENOENT') {
        throw error;
      }
    }
  }

  async loadRange(keyPrefix: StorageKey): Promise<Chunk[]> {
    const fsChunks = await this.#fsStorage.loadRange(keyPrefix);
    const bundleChunks = await this.#bundleStorage.loadRange(keyPrefix);

    const chunkMap = new Map<string, Chunk>();

    for (const chunk of bundleChunks) {
      const keyStr = chunk.key.join('/');
      chunkMap.set(keyStr, chunk);
    }

    for (const chunk of fsChunks) {
      const keyStr = chunk.key.join('/');
      chunkMap.set(keyStr, chunk);
    }

    return Array.from(chunkMap.values());
  }

  async removeRange(keyPrefix: StorageKey): Promise<void> {
    await this.#fsStorage.removeRange(keyPrefix);
  }

  getRootId(): string | null {
    return this.#bundleStorage.getRootId();
  }

  async createSlimBundle(
    newManifest?: Partial<ReturnType<BundleStorageAdapter['createSlimBundle']>>
  ): Promise<Uint8Array> {
    return await this.#bundleStorage.createSlimBundle(newManifest as any);
  }

  log(): void {
    console.log('CompositeStorageAdapter state:');
    console.log('Bundle storage:');
    this.#bundleStorage.log();
    console.log('\nFilesystem storage: Using NodeFSStorageAdapter');
  }
}
