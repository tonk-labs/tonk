import {describe, it, expect, beforeEach, afterEach, vi} from 'vitest';
import {SyncedFileManager} from '../../src/fs/syncedFileManager';
import {SyncEngine} from '../../src/engine';
import {FileMetadata} from '../../src/fs/types';

// Mock dependencies
vi.mock('../../src/engine');
vi.mock('../../src/fs/fileDoc', () => {
  return {
    AutomergeFileManager: class MockAutomergeFileManager {
      options: any;
      initialized = false;
      blobs: Map<string, Blob> = new Map();

      constructor(options: any) {
        this.options = options;
      }

      async init() {
        this.initialized = true;
      }

      async addFile(file: File): Promise<FileMetadata> {
        const hash = `hash-${file.name}`;
        await this.storeBlob(hash, file);
        return {
          name: file.name,
          hash,
          size: file.size,
          type: file.type,
          lastModified: file.lastModified,
        };
      }

      async getBlob(hash: string): Promise<Blob | null> {
        return this.blobs.get(hash) || null;
      }

      async hasBlob(hash: string): Promise<boolean> {
        return this.blobs.has(hash);
      }

      async storeBlob(hash: string, blob: Blob): Promise<void> {
        this.blobs.set(hash, blob);
      }

      async deleteBlob(hash: string): Promise<void> {
        this.blobs.delete(hash);
      }

      async getMissingBlobs(doc: any): Promise<string[]> {
        if (!doc.files) return [];
        return doc.files
          .map((file: FileMetadata) => file.hash)
          .filter((hash: string) => !this.blobs.has(hash));
      }

      close() {
        this.initialized = false;
      }
    },
  };
});

// Mock File
class MockFile implements File {
  name: string;
  lastModified: number;
  webkitRelativePath: string;
  private blob: Blob;
  async bytes(): Promise<Uint8Array> {
    return this.arrayBuffer().then(buffer => new Uint8Array(buffer));
  }

  constructor(parts: BlobPart[], name: string, options: FilePropertyBag = {}) {
    this.blob = new Blob(parts, options);
    this.name = name;
    this.lastModified = options.lastModified || Date.now();
    this.webkitRelativePath = '';
  }

  get size(): number {
    return this.blob.size;
  }

  get type(): string {
    return this.blob.type;
  }

  slice(start?: number, end?: number, contentType?: string): Blob {
    return this.blob.slice(start, end, contentType);
  }

  async arrayBuffer(): Promise<ArrayBuffer> {
    return this.blob.arrayBuffer();
  }

  async text(): Promise<string> {
    return this.blob.text();
  }

  stream(): ReadableStream<Uint8Array> {
    return this.blob.stream() as ReadableStream<Uint8Array>;
  }
}

describe('SyncedFileManager', () => {
  let syncedFileManager: SyncedFileManager;
  let mockSyncEngine: any;
  let registeredCallbacks: Map<string, Function> = new Map();
  let registeredMessageHandlers: Map<string, Function> = new Map();

  beforeEach(() => {
    // Reset mocks
    vi.clearAllMocks();
    registeredCallbacks = new Map();
    registeredMessageHandlers = new Map();

    // Setup mock SyncEngine
    mockSyncEngine = {
      getDocument: vi.fn(),
      createDocument: vi.fn(),
      updateDocument: vi.fn(),
      registerSyncCallback: vi.fn(callback => {
        registeredCallbacks.set('sync', callback);
      }),
      unregisterSyncCallback: vi.fn(),
      registerMessageHandler: vi.fn((type, handler) => {
        registeredMessageHandlers.set(type, handler);
      }),
      unregisterMessageHandler: vi.fn(),
      sendMessage: vi.fn(),
    };

    // Create SyncedFileManager instance
    syncedFileManager = new SyncedFileManager(
      mockSyncEngine as unknown as SyncEngine,
      'test-files',
    );

    // Mock global functions
    global.atob = vi.fn(str => Buffer.from(str, 'base64').toString('binary'));
    global.btoa = vi.fn(str => Buffer.from(str, 'binary').toString('base64'));
  });

  afterEach(() => {
    syncedFileManager.close();
  });

  describe('initialization', () => {
    it('should initialize and create document if it does not exist', async () => {
      mockSyncEngine.getDocument.mockResolvedValueOnce(null);

      await syncedFileManager.init();

      expect(mockSyncEngine.getDocument).toHaveBeenCalledWith('test-files');
      expect(mockSyncEngine.createDocument).toHaveBeenCalledWith('test-files', {
        files: [],
      });
      expect(mockSyncEngine.registerSyncCallback).toHaveBeenCalled();
      expect(mockSyncEngine.registerMessageHandler).toHaveBeenCalledWith(
        'blob_request',
        expect.any(Function),
      );
      expect(mockSyncEngine.registerMessageHandler).toHaveBeenCalledWith(
        'blob_response',
        expect.any(Function),
      );
    });

    it('should not create document if it already exists', async () => {
      mockSyncEngine.getDocument.mockResolvedValueOnce({files: []});

      await syncedFileManager.init();

      expect(mockSyncEngine.getDocument).toHaveBeenCalledWith('test-files');
      expect(mockSyncEngine.createDocument).not.toHaveBeenCalled();
    });

    it('should not initialize twice', async () => {
      mockSyncEngine.getDocument.mockResolvedValueOnce({files: []});

      await syncedFileManager.init();
      await syncedFileManager.init();

      expect(mockSyncEngine.getDocument).toHaveBeenCalledTimes(2);
    });
  });

  describe('file operations', () => {
    beforeEach(async () => {
      mockSyncEngine.getDocument.mockResolvedValue({files: []});
      await syncedFileManager.init();
    });

    it('should add a file and update the document', async () => {
      const file = new MockFile(['test content'], 'test.txt', {
        type: 'text/plain',
      });
      mockSyncEngine.updateDocument.mockImplementationOnce(
        (_docId: string, updateFn: (doc: any) => void) => {
          const doc = {files: []};
          updateFn(doc);
          return Promise.resolve(doc);
        },
      );

      const metadata = await syncedFileManager.addFile(file as unknown as File);

      expect(metadata).toEqual({
        name: 'test.txt',
        hash: 'hash-test.txt',
        size: file.size,
        type: 'text/plain',
        lastModified: expect.any(Number),
      });
      expect(mockSyncEngine.updateDocument).toHaveBeenCalledWith(
        'test-files',
        expect.any(Function),
      );
    });

    it('should remove a file and update the document', async () => {
      const hash = 'hash-to-remove';
      mockSyncEngine.updateDocument.mockImplementationOnce(
        (_docId: string, updateFn: (doc: any) => void) => {
          const doc = {files: [{hash, name: 'test.txt'}]};
          updateFn(doc);
          return Promise.resolve(doc);
        },
      );

      await syncedFileManager.removeFile(hash);

      expect(mockSyncEngine.updateDocument).toHaveBeenCalledWith(
        'test-files',
        expect.any(Function),
      );
    });

    it('should get all files from the document', async () => {
      const files = [
        {hash: 'hash1', name: 'file1.txt'},
        {hash: 'hash2', name: 'file2.txt'},
      ];
      mockSyncEngine.getDocument.mockResolvedValueOnce({files});

      const result = await syncedFileManager.getAllFiles();

      expect(result).toEqual(files);
    });

    it('should return empty array if document has no files', async () => {
      mockSyncEngine.getDocument.mockResolvedValueOnce({});

      const result = await syncedFileManager.getAllFiles();

      expect(result).toEqual([]);
    });
  });

  describe('blob synchronization', () => {
    beforeEach(async () => {
      mockSyncEngine.getDocument.mockResolvedValue({files: []});
      await syncedFileManager.init();
    });

    it('should handle document sync by checking for missing blobs', async () => {
      const syncCallback = registeredCallbacks.get('sync');
      const checkForMissingBlobsSpy = vi.spyOn(
        syncedFileManager as any,
        'checkForMissingBlobs',
      );

      mockSyncEngine.getDocument.mockResolvedValueOnce({
        files: [{hash: 'missing-hash', name: 'missing.txt'}],
      });

      if (syncCallback) {
        await syncCallback('test-files');
      }

      expect(checkForMissingBlobsSpy).toHaveBeenCalled();
    });

    it('should request missing blobs', async () => {
      // Mock the requestBlob method to avoid waiting for a response
      vi.spyOn(syncedFileManager as any, 'requestBlob').mockImplementation(
        function (hash: string) {
          // Just send the message without waiting for a response
          (this as any).syncEngine.sendMessage({
            type: 'blob_request',
            hash,
          });
          return Promise.resolve();
        },
      );

      mockSyncEngine.getDocument.mockResolvedValueOnce({
        files: [{hash: 'missing-hash', name: 'missing.txt'}],
      });

      await (syncedFileManager as any).checkForMissingBlobs();

      // Verify the message was sent
      expect(mockSyncEngine.sendMessage).toHaveBeenCalledWith({
        type: 'blob_request',
        hash: 'missing-hash',
      });
    });

    it('should handle blob requests by sending blob if available', async () => {
      const blobRequestHandler = registeredMessageHandlers.get('blob_request');
      const blob = new Blob(['test content'], {type: 'text/plain'});
      const hash = 'test-hash';

      // Mock that we have the blob
      vi.spyOn(syncedFileManager as any, 'hasBlob').mockResolvedValueOnce(true);
      vi.spyOn(syncedFileManager as any, 'getBlob').mockResolvedValueOnce(blob);
      vi.spyOn(
        syncedFileManager as any,
        'arrayBufferToBase64',
      ).mockReturnValueOnce('base64data');

      if (blobRequestHandler) {
        await blobRequestHandler({hash});
      }

      expect(mockSyncEngine.sendMessage).toHaveBeenCalledWith({
        type: 'blob_response',
        hash,
        blob: {
          data: 'base64data',
          type: 'text/plain',
        },
      });
    });

    it('should handle blob responses by storing the blob', async () => {
      const blobResponseHandler =
        registeredMessageHandlers.get('blob_response');
      const hash = 'received-hash';
      const storeBlobSpy = vi.spyOn(syncedFileManager as any, 'storeBlob');
      vi.spyOn(
        syncedFileManager as any,
        'base64ToArrayBuffer',
      ).mockReturnValueOnce(new ArrayBuffer(10));

      // Setup a handler in the request map
      const handlerMock = vi.fn();
      (syncedFileManager as any).blobRequestHandlers.set(hash, handlerMock);

      if (blobResponseHandler) {
        await blobResponseHandler({
          hash,
          blob: {
            data: 'base64data',
            type: 'text/plain',
          },
        });
      }

      expect(storeBlobSpy).toHaveBeenCalled();
      expect(handlerMock).toHaveBeenCalledWith(hash);
    });

    it('should sync missing blobs from another file manager', async () => {
      const otherManager = {
        getBlob: vi.fn().mockImplementation(hash => {
          if (hash === 'hash1') {
            return new Blob(['content1'], {type: 'text/plain'});
          }
          return null;
        }),
      };

      mockSyncEngine.getDocument.mockResolvedValueOnce({
        files: [
          {hash: 'hash1', name: 'file1.txt'},
          {hash: 'hash2', name: 'file2.txt'},
        ],
      });

      vi.spyOn(
        syncedFileManager as any,
        'getMissingBlobs',
      ).mockResolvedValueOnce(['hash1', 'hash2']);
      const storeBlobSpy = vi.spyOn(syncedFileManager as any, 'storeBlob');

      const result = await syncedFileManager.syncMissingBlobs(
        otherManager as any,
      );

      expect(result).toEqual(['hash1']);
      expect(storeBlobSpy).toHaveBeenCalledWith('hash1', expect.any(Blob));
    });
  });

  describe('utility methods', () => {
    it('should convert ArrayBuffer to base64', async () => {
      const buffer = new Uint8Array([1, 2, 3, 4]).buffer;
      await (syncedFileManager as any).arrayBufferToBase64(buffer);
      expect(global.btoa).toHaveBeenCalled();
    });

    it('should convert base64 to ArrayBuffer', async () => {
      const result = await (syncedFileManager as any).base64ToArrayBuffer(
        'dGVzdA==',
      );
      expect(global.atob).toHaveBeenCalledWith('dGVzdA==');
      expect(result).toBeInstanceOf(ArrayBuffer);
    });
  });

  describe('cleanup', () => {
    it('should unregister callbacks and handlers on close', async () => {
      mockSyncEngine.getDocument.mockResolvedValue({files: []});
      await syncedFileManager.init();

      syncedFileManager.close();

      expect(mockSyncEngine.unregisterSyncCallback).toHaveBeenCalled();
      expect(mockSyncEngine.unregisterMessageHandler).toHaveBeenCalledWith(
        'blob_request',
      );
      expect(mockSyncEngine.unregisterMessageHandler).toHaveBeenCalledWith(
        'blob_response',
      );
    });
  });
});
