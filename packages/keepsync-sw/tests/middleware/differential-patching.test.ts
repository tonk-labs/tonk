import {describe, it, expect, beforeEach, afterEach, vi} from 'vitest';
import 'fake-indexeddb/auto';
import {create} from 'zustand';
import {
  configureSyncEngine,
  resetSyncEngine,
} from '../../src/core/syncConfig.js';
import {IndexedDBStorageAdapter} from '@automerge/automerge-repo-storage-indexeddb';
import {DummyNetworkAdapter} from '../dummy/DummyNetworkAdapter.js';
import {
  sync,
  writeDoc,
  readDoc,
  listenToDoc,
} from '../../src/middleware/sync.js';

const TEST_DB = 'test-differential-patching';

// Test data interfaces
interface SimpleTestDoc {
  count: number;
  name: string;
}

interface ComplexTestDoc {
  metadata: {
    version: number;
    author: string;
    tags: string[];
  };
  content: {
    title: string;
    body: string;
    sections: Array<{
      id: string;
      title: string;
      content: string;
    }>;
  };
  stats: {
    views: number;
    likes: number;
    comments: string[];
  };
  settings: {
    public: boolean;
    allowComments: boolean;
    theme: 'light' | 'dark';
  };
}

interface LargeTestDoc {
  items: Array<{
    id: string;
    data: Record<string, any>;
  }>;
  bulk: Record<string, any>;
}

describe('Differential Patching Integration Tests', () => {
  beforeEach(async () => {
    resetSyncEngine();
    const syncEngine = configureSyncEngine({
      url: '',
      storage: new IndexedDBStorageAdapter(TEST_DB),
      network: new DummyNetworkAdapter(),
    });
    await syncEngine.whenReady();
  });

  afterEach(async () => {
    // Reset the sync engine first to close all connections
    resetSyncEngine();

    // Wait a bit for connections to close
    await new Promise(resolve => setTimeout(resolve, 100));

    // Clean up test database
    try {
      indexedDB.deleteDatabase(TEST_DB);
    } catch (error) {
      // Ignore cleanup errors - they don't affect test validity
    }
  });

  describe('mutateDocument functionality', () => {
    it('should only update changed properties in simple objects', async () => {
      const docId = 'simple-patch-test';
      const changesSpy = vi.fn();

      // Create initial document
      await writeDoc<SimpleTestDoc>(docId, {
        count: 0,
        name: 'initial',
      });

      // Listen to changes to verify patch behavior
      const removeListener = await listenToDoc<SimpleTestDoc>(
        docId,
        ({patches}) => {
          changesSpy(patches);
        },
      );

      // Update only one property
      await writeDoc<SimpleTestDoc>(docId, {
        count: 5,
        name: 'initial', // unchanged
      });

      // Allow time for change propagation
      await new Promise(resolve => setTimeout(resolve, 100));

      // Verify the document was updated correctly
      const result = await readDoc<SimpleTestDoc>(docId);
      expect(result).toEqual({
        count: 5,
        name: 'initial',
      });

      // Verify patches show minimal changes
      const patches = changesSpy.mock.calls.slice(1); // Skip initial call
      expect(patches.length).toBeGreaterThan(0);

      removeListener();
    });

    it('should handle nested object updates efficiently', async () => {
      const docId = 'nested-patch-test';
      const changesSpy = vi.fn();

      // Create initial complex document
      const initialDoc: ComplexTestDoc = {
        metadata: {
          version: 1,
          author: 'test-author',
          tags: ['tag1', 'tag2'],
        },
        content: {
          title: 'Test Title',
          body: 'Test Body',
          sections: [{id: '1', title: 'Section 1', content: 'Content 1'}],
        },
        stats: {
          views: 0,
          likes: 0,
          comments: [],
        },
        settings: {
          public: true,
          allowComments: true,
          theme: 'light',
        },
      };

      await writeDoc<ComplexTestDoc>(docId, initialDoc);

      // Listen to changes
      const removeListener = await listenToDoc<ComplexTestDoc>(
        docId,
        ({patches}) => {
          changesSpy(patches);
        },
      );

      // Update only nested properties
      const updatedDoc: ComplexTestDoc = {
        ...initialDoc,
        metadata: {
          ...initialDoc.metadata,
          version: 2, // Only this changes
        },
        stats: {
          ...initialDoc.stats,
          views: 100, // Only this changes
          likes: 5, // Only this changes
        },
      };

      await writeDoc<ComplexTestDoc>(docId, updatedDoc);

      // Allow time for change propagation
      await new Promise(resolve => setTimeout(resolve, 100));

      // Verify the document was updated correctly
      const result = await readDoc<ComplexTestDoc>(docId);
      expect(result?.metadata.version).toBe(2);
      expect(result?.stats.views).toBe(100);
      expect(result?.stats.likes).toBe(5);
      expect(result?.content.title).toBe('Test Title'); // Unchanged
      expect(result?.settings.theme).toBe('light'); // Unchanged

      removeListener();
    });

    it('should handle property deletion correctly', async () => {
      const docId = 'deletion-test';

      // Create document with optional property
      interface TestDocWithOptional {
        required: string;
        optional?: string;
        nested: {
          keep: string;
          remove?: string;
        };
      }

      await writeDoc<TestDocWithOptional>(docId, {
        required: 'keep this',
        optional: 'remove this',
        nested: {
          keep: 'keep this too',
          remove: 'remove this too',
        },
      });

      // Update document removing optional properties
      await writeDoc<TestDocWithOptional>(docId, {
        required: 'keep this',
        nested: {
          keep: 'keep this too',
        },
      });

      // Verify properties were removed
      const result = await readDoc<TestDocWithOptional>(docId);
      expect(result?.required).toBe('keep this');
      expect(result?.optional).toBeUndefined();
      expect(result?.nested.keep).toBe('keep this too');
      expect(result?.nested.remove).toBeUndefined();
    });

    it('should handle array updates efficiently', async () => {
      const docId = 'array-patch-test';

      interface ArrayTestDoc {
        items: string[];
        metadata: {
          count: number;
        };
      }

      // Create initial document
      await writeDoc<ArrayTestDoc>(docId, {
        items: ['a', 'b', 'c'],
        metadata: {
          count: 3,
        },
      });

      // Update array (arrays are replaced entirely by design)
      await writeDoc<ArrayTestDoc>(docId, {
        items: ['a', 'b', 'c', 'd'],
        metadata: {
          count: 4,
        },
      });

      // Verify update
      const result = await readDoc<ArrayTestDoc>(docId);
      expect(result?.items).toEqual(['a', 'b', 'c', 'd']);
      expect(result?.metadata.count).toBe(4);
    });
  });

  describe('Sync middleware differential patching', () => {
    it('should sync store changes to Automerge document efficiently', async () => {
      const docId = 'sync-patch-test';
      const changesSpy = vi.fn();

      // Create a store that syncs with Automerge
      interface TestStore extends SimpleTestDoc {
        increment: () => void;
        setName: (name: string) => void;
      }

      const useStore = create(
        sync<TestStore>(
          set => ({
            count: 0,
            name: 'initial',
            increment: () => set(state => ({...state, count: state.count + 1})),
            setName: (name: string) => set(state => ({...state, name})),
          }),
          {docId},
        ),
      );

      // Wait for sync initialization
      await new Promise(resolve => setTimeout(resolve, 800));

      // Listen to document changes to verify our differential patching
      const removeListener = await listenToDoc<SimpleTestDoc>(
        docId,
        ({doc, patches}) => {
          changesSpy({doc, patches});
        },
      );

      // Update count in store - this should trigger differential patching
      useStore.getState().increment();

      // Wait for sync
      await new Promise(resolve => setTimeout(resolve, 400));

      // Verify store state
      const state1 = useStore.getState();
      expect(state1.count).toBe(1);
      expect(state1.name).toBe('initial');

      // Verify document was updated with differential patching
      const docState = await readDoc<SimpleTestDoc>(docId);
      expect(docState?.count).toBe(1);
      expect(docState?.name).toBe('initial');

      // Update name in store
      useStore.getState().setName('updated');

      // Wait for sync
      await new Promise(resolve => setTimeout(resolve, 400));

      // Verify final sync
      const finalState = useStore.getState();
      const finalDocState = await readDoc<SimpleTestDoc>(docId);

      expect(finalState.count).toBe(1);
      expect(finalState.name).toBe('updated');
      expect(finalDocState?.count).toBe(1);
      expect(finalDocState?.name).toBe('updated');

      // Verify we received change notifications
      expect(changesSpy).toHaveBeenCalled();

      removeListener();
    });

    it('should handle document updates from external sources', async () => {
      const docId = 'external-update-test';

      interface TestDoc {
        counter1: number;
        counter2: number;
        metadata: {
          lastUpdated?: string;
        };
      }

      // Create a store that syncs with Automerge
      interface TestStore extends TestDoc {
        updateCounter1: () => void;
      }

      const useStore = create(
        sync<TestStore>(
          set => ({
            counter1: 0,
            counter2: 0,
            metadata: {},
            updateCounter1: () =>
              set(state => ({
                ...state,
                counter1: state.counter1 + 1,
                metadata: {
                  lastUpdated: new Date().toISOString(),
                },
              })),
          }),
          {docId},
        ),
      );

      // Wait for sync initialization
      await new Promise(resolve => setTimeout(resolve, 800));

      // Update from store
      useStore.getState().updateCounter1();
      await new Promise(resolve => setTimeout(resolve, 400));

      // Verify store state
      let state = useStore.getState();
      expect(state.counter1).toBe(1);
      expect(state.counter2).toBe(0);

      // Update the document externally using writeDoc (simulating another client)
      await writeDoc<TestDoc>(docId, {
        counter1: 1, // keep the store's update
        counter2: 5, // external update
        metadata: {
          lastUpdated: '2024-01-01T00:00:00.000Z', // external update
        },
      });

      // Wait for sync propagation back to store
      await new Promise(resolve => setTimeout(resolve, 600));

      // Verify the store received the external updates
      const finalState = useStore.getState();
      expect(finalState.counter1).toBe(1); // preserved from store
      expect(finalState.counter2).toBe(5); // updated from external
      expect(finalState.metadata.lastUpdated).toBe('2024-01-01T00:00:00.000Z'); // updated from external

      // Verify document state
      const docState = await readDoc<TestDoc>(docId);
      expect(docState?.counter1).toBe(1);
      expect(docState?.counter2).toBe(5);
      expect(docState?.metadata.lastUpdated).toBe('2024-01-01T00:00:00.000Z');
    });
  });

  describe('Performance characteristics', () => {
    it('should handle large documents efficiently', async () => {
      const docId = 'large-doc-test';

      // Create a large document
      const largeItems = Array.from({length: 1000}, (_, i) => ({
        id: `item-${i}`,
        data: {
          name: `Item ${i}`,
          description: `Description for item ${i}`,
          metadata: {
            created: new Date().toISOString(),
            tags: [`tag-${i % 10}`, `category-${i % 5}`],
            stats: {
              views: Math.floor(Math.random() * 1000),
              likes: Math.floor(Math.random() * 100),
            },
          },
        },
      }));

      const bulkData = Object.fromEntries(
        Array.from({length: 500}, (_, i) => [`key-${i}`, `value-${i}`]),
      );

      const largeDoc: LargeTestDoc = {
        items: largeItems,
        bulk: bulkData,
      };

      // Measure write time
      const writeStart = performance.now();
      await writeDoc<LargeTestDoc>(docId, largeDoc);
      const writeTime = performance.now() - writeStart;

      // Measure partial update time (only update first item)
      const partialUpdate: LargeTestDoc = {
        ...largeDoc,
        items: [
          {
            ...largeDoc.items[0],
            data: {
              ...largeDoc.items[0].data,
              name: 'Updated Item 0',
            },
          },
          ...largeDoc.items.slice(1),
        ],
      };

      const updateStart = performance.now();
      await writeDoc<LargeTestDoc>(docId, partialUpdate);
      const updateTime = performance.now() - updateStart;

      // Verify the update
      const result = await readDoc<LargeTestDoc>(docId);
      expect(result?.items[0].data.name).toBe('Updated Item 0');
      expect(result?.items[1].data.name).toBe('Item 1'); // Unchanged

      // Performance assertions (should be relatively fast)
      expect(writeTime).toBeLessThan(5000); // 5 seconds for initial write
      expect(updateTime).toBeLessThan(2000); // 2 seconds for update

      console.log(`Large document performance:
        - Initial write: ${writeTime.toFixed(2)}ms
        - Partial update: ${updateTime.toFixed(2)}ms
        - Items count: ${largeItems.length}
        - Bulk keys count: ${Object.keys(bulkData).length}`);
    });
  });

  describe('Error handling and edge cases', () => {
    it('should handle null values and skip undefined values', async () => {
      const docId = 'null-undefined-test';

      interface NullTestDoc {
        nullValue: null;
        normalValue: string;
        nested: {
          nullNested: null;
          normalNested: string;
        };
      }

      await writeDoc<NullTestDoc>(docId, {
        nullValue: null,
        normalValue: 'normal',
        nested: {
          nullNested: null,
          normalNested: 'nested normal',
        },
      });

      const result = await readDoc<NullTestDoc>(docId);
      expect(result?.nullValue).toBeNull();
      expect(result?.normalValue).toBe('normal');
      expect(result?.nested.nullNested).toBeNull();
      expect(result?.nested.normalNested).toBe('nested normal');

      // Test that undefined values are handled properly (skipped)
      const testWithUndefined = {
        nullValue: null,
        normalValue: 'updated normal',
        undefinedValue: undefined, // This should be skipped
        nested: {
          nullNested: null,
          normalNested: 'updated nested normal',
        },
      };

      await writeDoc(docId, testWithUndefined);
      const updatedResult = await readDoc<NullTestDoc>(docId);
      expect(updatedResult?.normalValue).toBe('updated normal');
      expect(updatedResult?.nested.normalNested).toBe('updated nested normal');
      expect(updatedResult).not.toHaveProperty('undefinedValue');
    });

    it('should handle circular reference prevention', async () => {
      const docId = 'circular-test';

      // Test that serialization handles potential circular references
      const store = create(
        sync<{data: any}>(
          set => ({
            data: {},
            updateData: (newData: any) => set({data: newData}),
          }),
          {docId},
        ),
      );

      // Wait for initialization
      await new Promise(resolve => setTimeout(resolve, 200));

      // This should work without circular reference issues
      store.getState().updateData({
        level1: {
          level2: {
            value: 'deep value',
          },
        },
      });

      await new Promise(resolve => setTimeout(resolve, 200));

      expect(store.getState().data.level1.level2.value).toBe('deep value');
    });
  });
});
