import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { IndexedDBStorageAdapter } from '@automerge/automerge-repo-storage-indexeddb';
import { listenToDoc, writeDoc } from '../../src/middleware/sync';
import { configureSyncEngine, resetSyncEngine } from '../../src/core/syncConfig';
import 'fake-indexeddb/auto';

describe('listenToDoc patch behavior', () => {
  const TEST_DB = 'test_listen_doc_db';
  const TEST_DOC_PATH = 'test/document';

  // Test data types
  interface TestDoc {
    count: number;
    items: string[];
    metadata?: {
      lastUpdated: string;
      version: number;
    };
  }

  beforeEach(async () => {
    // Reset the singleton instance to ensure clean test state
    resetSyncEngine();

    // Configure the sync engine with a test database
    // Use empty URL to create a local root document instead of fetching from server
    const syncEngine = await configureSyncEngine({
      url: '', // Empty URL forces creation of local root document
      storage: new IndexedDBStorageAdapter(TEST_DB),
    });

    // Wait for the sync engine to be ready
    await syncEngine.whenReady();
  });

  afterEach(async () => {
    // Clean up the singleton instance
    resetSyncEngine();

    // Clean up the test database
    indexedDB.deleteDatabase(TEST_DB);
  });

  it('should provide initial document state with empty patches on first listen', async () => {
    // Arrange: Create a document with initial state
    const initialData: TestDoc = {
      count: 5,
      items: ['a', 'b', 'c'],
      metadata: {
        lastUpdated: '2024-01-01',
        version: 1,
      },
    };

    await writeDoc(TEST_DOC_PATH, initialData);

    // Act: Listen to the document
    const listenerMock = vi.fn();
    const removeListener = await listenToDoc(TEST_DOC_PATH, listenerMock);

    // Assert: Should be called immediately with initial state
    expect(listenerMock).toHaveBeenCalledTimes(1);

    const initialCall = listenerMock.mock.calls[0][0];
    expect(initialCall).toHaveProperty('doc');
    expect(initialCall).toHaveProperty('patches');
    expect(initialCall).toHaveProperty('patchInfo');
    expect(initialCall).toHaveProperty('handle');

    // Check the document content
    expect(initialCall.doc).toEqual(initialData);

    // Check that patches are empty for initial call
    expect(initialCall.patches).toEqual([]);

    // Check patchInfo structure for initial call
    expect(initialCall.patchInfo).toEqual({
      before: null,
      after: initialData,
      source: 'initial',
    });

    // Check that handle is provided
    expect(initialCall.handle).toBeDefined();
    expect(typeof initialCall.handle.change).toBe('function');

    // Cleanup
    removeListener();
  });

  it('should provide patches when document changes externally', async () => {
    // Arrange: Create initial document
    const initialData: TestDoc = {
      count: 0,
      items: [],
    };

    await writeDoc(TEST_DOC_PATH, initialData);

    const listenerMock = vi.fn();
    const removeListener = await listenToDoc(TEST_DOC_PATH, listenerMock);

    // Clear the initial call
    listenerMock.mockClear();

    // Act: Make changes to the document externally
    const updatedData: TestDoc = {
      count: 3,
      items: ['x', 'y'],
      metadata: {
        lastUpdated: '2024-01-02',
        version: 2,
      },
    };

    await writeDoc(TEST_DOC_PATH, updatedData);

    // Wait a bit for the change to propagate
    await new Promise(resolve => setTimeout(resolve, 100));

    // Assert: Should receive change notification with patches
    expect(listenerMock).toHaveBeenCalled();

    const changeCall =
      listenerMock.mock.calls[listenerMock.mock.calls.length - 1][0];

    // Check the updated document content
    expect(changeCall.doc).toEqual(updatedData);

    // Check that patches are provided (should not be empty for real changes)
    expect(Array.isArray(changeCall.patches)).toBe(true);

    // Check patchInfo structure
    expect(changeCall.patchInfo).toBeDefined();
    expect(changeCall.patchInfo).toHaveProperty('before');
    expect(changeCall.patchInfo).toHaveProperty('after');
    expect(changeCall.patchInfo.after).toEqual(updatedData);

    // Check that handle is still provided
    expect(changeCall.handle).toBeDefined();

    // Cleanup
    removeListener();
  });

  it('should handle multiple sequential changes with correct patch information', async () => {
    // Arrange: Create initial document
    const initialData: TestDoc = {
      count: 1,
      items: ['initial'],
    };

    await writeDoc(TEST_DOC_PATH, initialData);

    const allCalls: Array<{
      doc: TestDoc;
      patches: any[];
      patchInfo: any;
      handle: any;
    }> = [];
    const listener = (payload: any) => {
      allCalls.push(payload);
    };

    const removeListener = await listenToDoc(TEST_DOC_PATH, listener);

    // Act: Make several sequential changes
    const changes = [
      { count: 2, items: ['initial', 'second'] },
      { count: 3, items: ['initial', 'second', 'third'] },
      { count: 3, items: ['updated', 'second', 'third'] },
    ];

    for (const change of changes) {
      await writeDoc(TEST_DOC_PATH, change);
      // Small delay to ensure changes are processed sequentially
      await new Promise(resolve => setTimeout(resolve, 50));
    }

    // Wait for all changes to propagate
    await new Promise(resolve => setTimeout(resolve, 200));

    // Assert: Should have received multiple calls (initial + changes)
    expect(allCalls.length).toBeGreaterThan(1);

    // Check the final state
    const finalCall = allCalls[allCalls.length - 1];
    expect(finalCall.doc.count).toBe(3);
    expect(finalCall.doc.items).toEqual(['updated', 'second', 'third']);

    // Each call should have the required structure
    allCalls.forEach((call, index) => {
      expect(call).toHaveProperty('doc');
      expect(call).toHaveProperty('patches');
      expect(call).toHaveProperty('patchInfo');
      expect(call).toHaveProperty('handle');

      // Initial call should have empty patches and special patchInfo
      if (index === 0) {
        expect(call.patches).toEqual([]);
        expect(call.patchInfo.source).toBe('initial');
        expect(call.patchInfo.before).toBe(null);
      }
    });

    // Cleanup
    removeListener();
  });

  it('should stop receiving updates after listener is removed', async () => {
    // Arrange: Create initial document
    const initialData: TestDoc = {
      count: 1,
      items: ['test'],
    };

    await writeDoc(TEST_DOC_PATH, initialData);

    const listenerMock = vi.fn();
    const removeListener = await listenToDoc(TEST_DOC_PATH, listenerMock);

    // Clear initial call
    listenerMock.mockClear();

    // Act: Remove the listener
    removeListener();

    // Make a change after removing listener
    await writeDoc(TEST_DOC_PATH, { count: 2, items: ['test', 'updated'] });

    // Wait for potential propagation
    await new Promise(resolve => setTimeout(resolve, 100));

    // Assert: Should not receive any more calls
    expect(listenerMock).not.toHaveBeenCalled();
  });

  it('should handle documents with nested objects correctly', async () => {
    // Arrange: Create document with complex nested structure
    interface ComplexDoc {
      user: {
        id: string;
        profile: {
          name: string;
          settings: {
            theme: string;
            notifications: boolean;
          };
        };
      };
      data: {
        items: Array<{ id: number; name: string; active: boolean }>;
        counters: Record<string, number>;
      };
    }

    const complexData: ComplexDoc = {
      user: {
        id: 'user-123',
        profile: {
          name: 'Test User',
          settings: {
            theme: 'dark',
            notifications: true,
          },
        },
      },
      data: {
        items: [
          { id: 1, name: 'Item 1', active: true },
          { id: 2, name: 'Item 2', active: false },
        ],
        counters: {
          views: 10,
          likes: 5,
        },
      },
    };

    await writeDoc(TEST_DOC_PATH, complexData);

    const listenerMock = vi.fn();
    const removeListener = await listenToDoc(TEST_DOC_PATH, listenerMock);

    // Clear initial call
    listenerMock.mockClear();

    // Act: Update nested properties
    const updatedData: ComplexDoc = {
      ...complexData,
      user: {
        ...complexData.user,
        profile: {
          ...complexData.user.profile,
          settings: {
            theme: 'light', // Changed
            notifications: false, // Changed
          },
        },
      },
      data: {
        ...complexData.data,
        counters: {
          views: 15, // Changed
          likes: 5,
          shares: 2, // Added
        },
      },
    };

    await writeDoc(TEST_DOC_PATH, updatedData);

    // Wait for change to propagate
    await new Promise(resolve => setTimeout(resolve, 100));

    // Assert: Should receive change with correct structure
    expect(listenerMock).toHaveBeenCalled();

    const changeCall = listenerMock.mock.calls[0][0];
    expect(changeCall.doc).toEqual(updatedData);
    expect(changeCall.doc.user.profile.settings.theme).toBe('light');
    expect(changeCall.doc.data.counters.shares).toBe(2);

    // Patches should be provided for nested changes
    expect(Array.isArray(changeCall.patches)).toBe(true);
    expect(changeCall.patchInfo).toBeDefined();

    // Cleanup
    removeListener();
  });

  it('should throw error when trying to listen to non-existent document', async () => {
    // Act & Assert: Should throw when document doesn't exist
    await expect(listenToDoc('non/existent/path', () => { })).rejects.toThrow(
      'Document not found at path: non/existent/path',
    );
  });

  it('should provide consistent handle reference across calls', async () => {
    // Arrange: Create initial document
    const initialData: TestDoc = {
      count: 1,
      items: ['test'],
    };

    await writeDoc(TEST_DOC_PATH, initialData);

    const handles: any[] = [];
    const listener = (payload: any) => {
      handles.push(payload.handle);
    };

    const removeListener = await listenToDoc(TEST_DOC_PATH, listener);

    // Act: Make changes to trigger multiple calls
    await writeDoc(TEST_DOC_PATH, { count: 2, items: ['test', 'change1'] });
    await new Promise(resolve => setTimeout(resolve, 50));
    await writeDoc(TEST_DOC_PATH, {
      count: 3,
      items: ['test', 'change1', 'change2'],
    });
    await new Promise(resolve => setTimeout(resolve, 100));

    // Assert: All handles should be the same reference
    expect(handles.length).toBeGreaterThan(1);
    const firstHandle = handles[0];
    handles.forEach(handle => {
      expect(handle).toBe(firstHandle);
    });

    // Cleanup
    removeListener();
  });
});
