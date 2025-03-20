import * as Automerge from '@automerge/automerge';
import * as fs from 'fs/promises';
import * as path from 'path';
import {DocumentId} from './types.js';

export interface FileSystemStorageOptions {
  storagePath: string;
  syncInterval?: number; // How often to sync to disk in milliseconds
  createIfMissing?: boolean; // Create directory if it doesn't exist
}

export class AutomergeFileSystemStorage {
  private documents: Map<DocumentId, Automerge.Doc<any>> = new Map();
  private savePending: Set<DocumentId> = new Set();
  private syncTimer: NodeJS.Timeout | null = null;
  private initialized = false;

  constructor(
    private options: FileSystemStorageOptions,
    private logFn: (
      color: 'green' | 'red' | 'blue' | 'yellow',
      message: string,
    ) => void,
  ) {
    this.initialize();
  }

  private log(
    color: 'green' | 'red' | 'blue' | 'yellow',
    message: string,
  ): void {
    this.logFn(color, message);
  }

  private async initialize(): Promise<void> {
    try {
      // Check if directory exists
      try {
        await fs.access(this.options.storagePath);
      } catch (error) {
        // Directory doesn't exist
        if (this.options.createIfMissing) {
          this.log(
            'yellow',
            `Creating storage directory: ${this.options.storagePath}`,
          );
          await fs.mkdir(this.options.storagePath, {recursive: true});
        } else {
          throw new Error(
            `Storage directory does not exist: ${this.options.storagePath}`,
          );
        }
      }

      // Load documents from storage
      await this.loadDocumentsFromFileSystem();

      // Set up periodic sync if interval provided
      if (this.options.syncInterval && this.options.syncInterval > 0) {
        this.syncTimer = setInterval(() => {
          this.syncPendingDocuments().catch(error => {
            this.log('red', `Error syncing documents: ${error.message}`);
          });
        }, this.options.syncInterval);
      }

      this.initialized = true;
      this.log(
        'green',
        `FileSystem storage initialized at ${this.options.storagePath}`,
      );
    } catch (error) {
      this.log(
        'red',
        `Failed to initialize FileSystem storage: ${(error as Error).message}`,
      );
      throw error;
    }
  }

  public isInitialized(): boolean {
    return this.initialized;
  }

  public async shutdown(): Promise<void> {
    if (this.syncTimer) {
      clearInterval(this.syncTimer);
      this.syncTimer = null;
    }

    // Final sync to ensure all pending documents are saved
    await this.syncPendingDocuments();
    this.log('green', 'FileSystem storage shut down');
  }

  public async loadDocumentsFromFileSystem(): Promise<void> {
    try {
      const files = await fs.readdir(this.options.storagePath);
      const docFiles = files.filter(file => file.endsWith('.automerge'));

      this.log('blue', `Found ${docFiles.length} documents in storage`);

      for (const file of docFiles) {
        try {
          const docId = path.basename(file, '.automerge');
          const filePath = path.join(this.options.storagePath, file);
          const docBuffer = await fs.readFile(filePath);

          // Load document from binary format
          const doc = Automerge.load(new Uint8Array(docBuffer));
          this.documents.set(docId, doc);
          this.log('blue', `Loaded document: ${docId}`);
        } catch (error) {
          this.log(
            'red',
            `Error loading document ${file}: ${(error as Error).message}`,
          );
        }
      }
    } catch (error) {
      this.log(
        'red',
        `Error loading documents from filesystem: ${(error as Error).message}`,
      );
      throw error;
    }
  }

  public getDocument(id: DocumentId): Automerge.Doc<any> | null {
    return this.documents.get(id) || null;
  }

  public storeDocument(id: DocumentId, doc: Automerge.Doc<any>): void {
    this.documents.set(id, doc);
    this.savePending.add(id);

    // If no sync interval is set, sync immediately
    if (!this.options.syncInterval || this.options.syncInterval <= 0) {
      this.syncDocument(id).catch(error => {
        this.log('red', `Error syncing document ${id}: ${error.message}`);
      });
    }
  }

  private async syncDocument(id: DocumentId): Promise<void> {
    try {
      const doc = this.documents.get(id);
      if (!doc) {
        this.log('yellow', `Document ${id} not found for syncing`);
        return;
      }

      const serialized = Automerge.save(doc);
      const filePath = path.join(this.options.storagePath, `${id}.automerge`);

      // Write to a temporary file first, then rename for atomic write
      const tempPath = `${filePath}.tmp`;
      await fs.writeFile(tempPath, Buffer.from(serialized));
      await fs.rename(tempPath, filePath);

      this.savePending.delete(id);
      this.log('blue', `Document ${id} synced to filesystem`);
    } catch (error) {
      this.log(
        'red',
        `Error syncing document ${id}: ${(error as Error).message}`,
      );
      throw error;
    }
  }

  private async syncPendingDocuments(): Promise<void> {
    const pending = Array.from(this.savePending);
    if (pending.length === 0) return;

    this.log(
      'blue',
      `Syncing ${pending.length} pending documents to filesystem`,
    );

    const syncPromises = pending.map(id => this.syncDocument(id));
    await Promise.all(syncPromises);
  }

  public async forceSyncToFileSystem(): Promise<void> {
    await this.syncPendingDocuments();
    this.log('green', 'Forced sync to filesystem completed');
  }

  public getAllDocumentIds(): string[] {
    return Array.from(this.documents.keys());
  }

  // Delete a document from storage
  public async deleteDocument(id: DocumentId): Promise<void> {
    if (!this.documents.has(id)) {
      return;
    }

    try {
      const filePath = path.join(this.options.storagePath, `${id}.automerge`);
      await fs.unlink(filePath);
      this.documents.delete(id);
      this.savePending.delete(id);
      this.log('blue', `Document ${id} deleted from filesystem`);
    } catch (error) {
      this.log(
        'red',
        `Error deleting document ${id}: ${(error as Error).message}`,
      );
      throw error;
    }
  }
}
