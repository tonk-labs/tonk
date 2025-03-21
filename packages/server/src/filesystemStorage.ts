import * as Automerge from '@automerge/automerge';
import * as fs from 'fs/promises';
import * as path from 'path';
import {DocumentId} from './types.js';
import * as os from 'os';

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
  private storagePath: string;

  constructor(
    private options: FileSystemStorageOptions,
    private logFn: (
      color: 'green' | 'red' | 'blue' | 'yellow',
      message: string,
    ) => void,
  ) {
    // Normalize the storage path immediately during construction
    this.storagePath = this.normalizePath(options.storagePath);
    this.log('blue', `Using normalized storage path: ${this.storagePath}`);
    this.initialize();
  }

  private log(
    color: 'green' | 'red' | 'blue' | 'yellow',
    message: string,
  ): void {
    this.logFn(color, message);
  }

  /**
   * Normalizes a file path to prevent permission issues:
   * - Expands tilde (~) to user's home directory
   * - Redirects absolute paths that might cause permission issues to a safe location
   */
  private normalizePath(inputPath: string): string {
    // Handle home directory expansion (~/path)
    if (inputPath.startsWith('~/')) {
      return path.join(os.homedir(), inputPath.substring(2));
    }

    // Redirect absolute paths that aren't in standard user directories to avoid permission issues
    if (
      inputPath.startsWith('/') &&
      !inputPath.startsWith('/home/') &&
      !inputPath.startsWith('/Users/') &&
      !inputPath.startsWith('/tmp/') &&
      !inputPath.startsWith('/var/') &&
      // Avoid redirecting path inside container volume mounts
      !inputPath.includes('node_modules') &&
      !inputPath.includes('/app/')
    ) {
      const safePath = path.join(os.homedir(), 'tonk-data');
      this.log(
        'yellow',
        `Redirecting potentially unsafe path ${inputPath} to ${safePath}`,
      );
      return safePath;
    }

    return inputPath;
  }

  private async initialize(): Promise<void> {
    try {
      this.log(
        'blue',
        `Initializing filesystem storage at: ${this.storagePath}`,
      );

      // Check if directory exists and is accessible
      try {
        await fs.access(this.storagePath);
        this.log(
          'blue',
          `Storage directory exists and is accessible: ${this.storagePath}`,
        );
      } catch (error) {
        // Directory doesn't exist or isn't accessible
        if (this.options.createIfMissing) {
          try {
            this.log(
              'yellow',
              `Creating storage directory: ${this.storagePath}`,
            );
            await fs.mkdir(this.storagePath, {recursive: true, mode: 0o755});
          } catch (mkdirError: any) {
            // Handle permission errors by falling back to a safer location
            if (mkdirError.code === 'EACCES' || mkdirError.code === 'EPERM') {
              const fallbackPath = path.join(os.homedir(), 'tonk-data');
              this.log(
                'yellow',
                `Permission error creating ${this.storagePath}, falling back to ${fallbackPath}`,
              );

              // Create the fallback directory
              try {
                await fs.mkdir(fallbackPath, {recursive: true, mode: 0o755});
                // Update the path to use the fallback
                this.storagePath = fallbackPath;
              } catch (fallbackError: any) {
                // If home directory fallback fails, try /tmp as a last resort
                if (
                  fallbackError.code === 'EACCES' ||
                  fallbackError.code === 'EPERM'
                ) {
                  const tmpPath = path.join('/tmp', 'tonk-data');
                  this.log(
                    'yellow',
                    `Permission error creating ${fallbackPath}, falling back to ${tmpPath}`,
                  );
                  await fs.mkdir(tmpPath, {recursive: true, mode: 0o755});
                  this.storagePath = tmpPath;
                } else {
                  throw fallbackError;
                }
              }
            } else {
              throw mkdirError;
            }
          }
        } else {
          throw new Error(
            `Storage directory does not exist: ${this.storagePath}`,
          );
        }
      }

      // Verify write permissions by writing a test file
      const testFilePath = path.join(this.storagePath, '.write-test');
      try {
        await fs.writeFile(testFilePath, 'test');
        await fs.unlink(testFilePath);
        this.log('blue', 'Storage directory is writable');
      } catch (writeError: any) {
        if (writeError.code === 'EACCES' || writeError.code === 'EPERM') {
          const fallbackPath = path.join('/tmp', 'tonk-data');
          this.log(
            'yellow',
            `Storage directory ${this.storagePath} is not writable, falling back to ${fallbackPath}`,
          );
          await fs.mkdir(fallbackPath, {recursive: true, mode: 0o755});
          this.storagePath = fallbackPath;
        } else {
          throw writeError;
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
        `FileSystem storage initialized at ${this.storagePath}`,
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
      const files = await fs.readdir(this.storagePath);
      const docFiles = files.filter(file => file.endsWith('.automerge'));

      this.log('blue', `Found ${docFiles.length} documents in storage`);

      for (const file of docFiles) {
        try {
          const docId = path.basename(file, '.automerge');
          const filePath = path.join(this.storagePath, file);
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
      const filePath = path.join(this.storagePath, `${id}.automerge`);

      // Write to a temporary file first, then rename for atomic write
      const tempPath = `${filePath}.tmp`;
      try {
        await fs.writeFile(tempPath, Buffer.from(serialized));
        await fs.rename(tempPath, filePath);
      } catch (writeError: any) {
        // If write failed due to permissions, try a fallback path
        if (writeError.code === 'EACCES' || writeError.code === 'EPERM') {
          // Create a temp directory if needed
          const tempDir = path.join('/tmp', 'tonk-data');
          try {
            await fs.mkdir(tempDir, {recursive: true});

            // Try to write to the temp directory instead
            const tempFilePath = path.join(tempDir, `${id}.automerge`);
            const tempTempPath = `${tempFilePath}.tmp`;

            await fs.writeFile(tempTempPath, Buffer.from(serialized));
            await fs.rename(tempTempPath, tempFilePath);

            this.log(
              'yellow',
              `Permission issues writing to ${filePath}, saved to ${tempFilePath} instead`,
            );

            // Update storage path for future operations
            this.storagePath = tempDir;
          } catch (fallbackError) {
            throw fallbackError;
          }
        } else {
          throw writeError;
        }
      }

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
      const filePath = path.join(this.storagePath, `${id}.automerge`);
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

  // Get the current storage path (useful for diagnostics)
  public getStoragePath(): string {
    return this.storagePath;
  }
}
