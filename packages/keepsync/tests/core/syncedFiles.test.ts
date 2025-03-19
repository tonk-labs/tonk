import {describe, it, expect, vi, beforeEach, afterEach} from 'vitest';
import {
  configureSyncedFileSystem,
  getSyncedFileManager,
  addFile,
  removeFile,
  getFile,
  getAllFiles,
  closeSyncedFileSystem,
} from '../../src/core/syncedFiles';
import {SyncedFileManager} from '../../src/fs/syncedFileManager';

// Mock the dependencies
vi.mock('../../src/fs/syncedFileManager', () => {
  return {
    SyncedFileManager: vi.fn().mockImplementation(() => ({
      init: vi.fn().mockResolvedValue(undefined),
      addFile: vi.fn().mockImplementation(file =>
        Promise.resolve({
          hash: 'test-hash',
          name: file.name,
          size: file.size,
          type: file.type,
        }),
      ),
      removeFile: vi.fn().mockResolvedValue(undefined),
      getBlob: vi
        .fn()
        .mockImplementation(hash =>
          hash === 'test-hash'
            ? Promise.resolve(new Blob(['test content'], {type: 'text/plain'}))
            : Promise.resolve(null),
        ),
      getAllFiles: vi
        .fn()
        .mockResolvedValue([
          {hash: 'test-hash', name: 'test.txt', size: 12, type: 'text/plain'},
        ]),
    })),
  };
});

vi.mock('../../src/core/syncConfig', () => {
  return {
    getSyncEngine: vi.fn().mockResolvedValue({
      // Mock sync engine methods as needed
      sendMessage: vi.fn(),
      onMessage: vi.fn(),
      getDocument: vi.fn(),
      updateDocument: vi.fn(),
    }),
  };
});

describe('Synced Files Module', () => {
  beforeEach(() => {
    // Reset the module state before each test
    closeSyncedFileSystem();
    vi.clearAllMocks();
  });

  afterEach(() => {
    // Clean up after each test
    closeSyncedFileSystem();
  });

  describe('configureSyncedFileSystem', () => {
    it('should not initialize the file manager immediately', async () => {
      configureSyncedFileSystem();

      // SyncedFileManager constructor should not be called yet
      expect(SyncedFileManager).not.toHaveBeenCalled();
    });

    it('should accept custom options', async () => {
      configureSyncedFileSystem({
        docId: 'custom-files',
        dbName: 'custom-db',
        storeName: 'custom-store',
      });

      // Get the manager to trigger initialization
      await getSyncedFileManager();

      // Verify the options were passed correctly
      expect(SyncedFileManager).toHaveBeenCalledWith(
        expect.anything(),
        'custom-files',
        {
          dbName: 'custom-db',
          storeName: 'custom-store',
        },
      );
    });
  });

  describe('getSyncedFileManager', () => {
    it('should initialize the file manager with default options when not configured', async () => {
      const manager = await getSyncedFileManager();

      expect(manager).toBeDefined();
      expect(SyncedFileManager).toHaveBeenCalledWith(
        expect.anything(),
        'files',
        {
          dbName: 'sync-engine-store',
          storeName: 'file-blobs',
        },
      );
    });

    it('should return the same instance on multiple calls', async () => {
      const manager1 = await getSyncedFileManager();
      const manager2 = await getSyncedFileManager();

      expect(manager1).toBe(manager2);
      expect(SyncedFileManager).toHaveBeenCalledTimes(1);
    });
  });

  describe('addFile', () => {
    it('should add a file to the synced file system', async () => {
      const file = new File(['test content'], 'test.txt', {type: 'text/plain'});
      const result = await addFile(file);

      expect(result).toEqual({
        hash: 'test-hash',
        name: 'test.txt',
        size: file.size,
        type: 'text/plain',
      });

      // Get the manager instance that was created
      const manager = await getSyncedFileManager();
      expect(manager!.addFile).toHaveBeenCalledWith(file);
    });
  });

  describe('removeFile', () => {
    it('should remove a file from the synced file system', async () => {
      await removeFile('test-hash');

      // Get the manager instance that was created
      const manager = await getSyncedFileManager();
      expect(manager!.removeFile).toHaveBeenCalledWith('test-hash');
    });
  });

  describe('getFile', () => {
    it('should get a file blob by its hash', async () => {
      const blob = await getFile('test-hash');

      expect(blob).toBeInstanceOf(Blob);
      const text = await blob?.text();
      expect(text).toBe('test content');

      // Get the manager instance that was created
      const manager = await getSyncedFileManager();
      expect(manager!.getBlob).toHaveBeenCalledWith('test-hash');
    });

    it('should return null for non-existent file', async () => {
      const blob = await getFile('non-existent-hash');

      expect(blob).toBeNull();

      // Get the manager instance that was created
      const manager = await getSyncedFileManager();
      expect(manager!.getBlob).toHaveBeenCalledWith('non-existent-hash');
    });
  });

  describe('getAllFiles', () => {
    it('should get all files in the synced file system', async () => {
      const files = await getAllFiles();

      expect(files).toEqual([
        {hash: 'test-hash', name: 'test.txt', size: 12, type: 'text/plain'},
      ]);

      // Get the manager instance that was created
      const manager = await getSyncedFileManager();
      expect(manager!.getAllFiles).toHaveBeenCalled();
    });
  });

  describe('closeSyncedFileSystem', () => {
    it('should reset the file manager state', async () => {
      // Initialize the manager first
      await getSyncedFileManager();

      // Close the file system
      closeSyncedFileSystem();

      // Re-initialize should create a new instance
      await getSyncedFileManager();

      // SyncedFileManager constructor should be called twice
      expect(SyncedFileManager).toHaveBeenCalledTimes(2);
    });
  });
});
