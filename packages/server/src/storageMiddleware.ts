import B2 from 'backblaze-b2';
import fs from 'fs';
import path from 'path';
import os from 'os';
import crypto from 'crypto';

export interface BackblazeStorageOptions {
  enabled: boolean;
  applicationKeyId: string;
  applicationKey: string;
  bucketId: string;
  bucketName: string;
  syncInterval?: number; // in milliseconds, defaults to 5 minutes
  maxRetries?: number;
  tempDir?: string;
}

export interface StorageMiddlewareOptions {
  backblaze?: BackblazeStorageOptions;
}

interface UploadUrlResponse {
  uploadUrl: string;
  authorizationToken: string;
}

export class StorageMiddleware {
  private options: StorageMiddlewareOptions;
  private b2Client: B2 | null = null;
  private uploadUrlCache: UploadUrlResponse | null = null;
  private syncTimer: NodeJS.Timeout | null = null;
  private documentCache: Map<string, Buffer> = new Map();
  private tempDir: string;
  private log: (
    color: 'green' | 'red' | 'blue' | 'yellow',
    message: string,
  ) => void;
  private verbose: boolean;

  constructor(
    options: StorageMiddlewareOptions,
    logFunction: (
      color: 'green' | 'red' | 'blue' | 'yellow',
      message: string,
    ) => void,
    verbose: boolean = true,
  ) {
    this.options = options;
    this.log = logFunction;
    this.verbose = verbose;
    this.tempDir =
      options.backblaze?.tempDir ||
      path.join(os.tmpdir(), 'tonk-backblaze-sync');

    // Create temp directory if it doesn't exist
    if (!fs.existsSync(this.tempDir)) {
      fs.mkdirSync(this.tempDir, {recursive: true});
    }

    // Initialize Backblaze client if enabled
    this.initBackblazeClient();
  }

  private initBackblazeClient(): void {
    const backblaze = this.options.backblaze;

    if (!backblaze || !backblaze.enabled) {
      return;
    }

    try {
      this.b2Client = new B2({
        applicationKeyId: backblaze.applicationKeyId,
        applicationKey: backblaze.applicationKey,
      });

      // Authenticate when initializing
      this.b2Client
        .authorize()
        .then(() => {
          this.log('green', 'Backblaze B2 client authenticated successfully');

          // Start sync timer
          this.startSyncTimer();
        })
        .catch(error => {
          this.log(
            'red',
            `Failed to authenticate with Backblaze B2: ${error.message}`,
          );
          this.b2Client = null;
        });
    } catch (error) {
      this.log('red', `Error initializing Backblaze B2 client: ${error}`);
      this.b2Client = null;
    }
  }

  private startSyncTimer(): void {
    if (!this.options.backblaze?.enabled || this.syncTimer) {
      return;
    }

    const syncInterval = this.options.backblaze.syncInterval || 5 * 60 * 1000; // Default: 5 minutes

    this.syncTimer = setInterval(() => {
      this.syncDocumentsToBackblaze().catch(error => {
        this.log('red', `Sync to Backblaze failed: ${error.message}`);
      });
    }, syncInterval);
  }

  private stopSyncTimer(): void {
    if (this.syncTimer) {
      clearInterval(this.syncTimer);
      this.syncTimer = null;
    }
  }

  // Handle WebSocket message to capture document data
  public handleMessage(message: Buffer): void {
    try {
      // Try to parse the message to see if it contains document data
      const data = JSON.parse(message.toString());

      // Check if this is an Automerge document message
      if (data.docId && data.changes) {
        // Store the latest document state
        this.documentCache.set(data.docId, message);

        if (this.verbose) {
          this.log('blue', `Cached document: ${data.docId}`);
        }
      }
    } catch (error) {
      // Not a JSON message or doesn't contain the expected fields
      // This is normal for binary messages or other protocols
    }
  }

  // Get upload URL from Backblaze (with caching)
  private async getUploadUrl(): Promise<UploadUrlResponse> {
    if (!this.b2Client) {
      throw new Error('Backblaze client not initialized');
    }

    if (this.uploadUrlCache) {
      return this.uploadUrlCache;
    }

    try {
      const response = await this.b2Client.getUploadUrl({
        bucketId: this.options.backblaze!.bucketId,
      });

      this.uploadUrlCache = response.data;
      return this.uploadUrlCache!;
    } catch (error) {
      // If we get an auth error, try to re-authenticate
      if (
        (error as Error).message.includes('unauthorized') ||
        (error as Error).message.includes('expired')
      ) {
        await this.b2Client.authorize();

        // Try again after re-auth
        const response = await this.b2Client.getUploadUrl({
          bucketId: this.options.backblaze!.bucketId,
        });

        this.uploadUrlCache = response.data;
        return this.uploadUrlCache!;
      }

      throw error;
    }
  }

  // Upload a document to Backblaze
  private async uploadDocument(docId: string, data: Buffer): Promise<void> {
    if (!this.b2Client) {
      throw new Error('Backblaze client not initialized');
    }

    try {
      // Get upload URL
      const uploadUrl = await this.getUploadUrl();

      // Create a content hash (SHA1 is required by Backblaze)
      const contentHash = crypto.createHash('sha1').update(data).digest('hex');

      // Write to temporary file
      const tempFilePath = path.join(this.tempDir, `${docId}.json`);
      fs.writeFileSync(tempFilePath, data);

      // Upload file
      await this.b2Client.uploadFile({
        uploadUrl: uploadUrl.uploadUrl,
        uploadAuthToken: uploadUrl.authorizationToken,
        fileName: `documents/${docId}.json`,
        data: fs.readFileSync(tempFilePath),
        hash: contentHash,
      });

      // Clean up temp file
      fs.unlinkSync(tempFilePath);

      this.log('green', `Document ${docId} uploaded to Backblaze B2`);
    } catch (error) {
      // Invalidate upload URL cache on error
      this.uploadUrlCache = null;

      // Re-throw error for handling
      throw error;
    }
  }

  // Sync all cached documents to Backblaze
  public async syncDocumentsToBackblaze(): Promise<void> {
    if (!this.options.backblaze?.enabled || !this.b2Client) {
      return;
    }

    if (this.documentCache.size === 0) {
      if (this.verbose) {
        this.log('blue', 'No documents to sync to Backblaze');
      }
      return;
    }

    this.log(
      'blue',
      `Syncing ${this.documentCache.size} documents to Backblaze B2...`,
    );

    const promises: Promise<void>[] = [];
    const maxRetries = this.options.backblaze.maxRetries || 3;

    for (const [docId, data] of this.documentCache.entries()) {
      const uploadWithRetry = async () => {
        let retries = 0;

        while (retries < maxRetries) {
          try {
            await this.uploadDocument(docId, data);
            return;
          } catch (error) {
            retries++;

            if (retries >= maxRetries) {
              this.log(
                'red',
                `Failed to upload document ${docId} after ${maxRetries} attempts: ${(error as Error).message}`,
              );
              throw error;
            }

            // Wait before retrying (exponential backoff)
            const delay = Math.pow(2, retries) * 1000;
            await new Promise(resolve => setTimeout(resolve, delay));
          }
        }
      };

      promises.push(uploadWithRetry());
    }

    try {
      await Promise.all(promises);
      this.log(
        'green',
        `Successfully synced ${this.documentCache.size} documents to Backblaze B2`,
      );
    } catch (error) {
      this.log(
        'red',
        `Error during document sync: ${(error as Error).message}`,
      );
      throw error;
    }
  }

  // Force an immediate sync
  public async forceSyncToBackblaze(): Promise<void> {
    return this.syncDocumentsToBackblaze();
  }

  // Clean up resources on shutdown
  public async shutdown(): Promise<void> {
    this.stopSyncTimer();

    // Force final sync before shutdown
    if (this.documentCache.size > 0) {
      try {
        await this.syncDocumentsToBackblaze();
      } catch (error) {
        this.log(
          'red',
          `Final sync failed during shutdown: ${(error as Error).message}`,
        );
      }
    }
  }
}
