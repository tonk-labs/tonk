import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { create } from 'zustand';
import { sync } from '../../src/middleware/sync';
import { configureSyncEngine, resetSyncEngine } from '../../src/core/syncConfig';
import { IndexedDBStorageAdapter } from '@automerge/automerge-repo-storage-indexeddb';
import * as Automerge from '@automerge/automerge';
import 'fake-indexeddb/auto';
import { setIsSlim } from '../../src/utils/wasmState';
import { readDoc } from '../../src/middleware/sync';

describe('initializeSync empty document fix', () => {
  beforeEach(async () => {
    setIsSlim();
    Automerge.init();
    resetSyncEngine();
    const engine = await configureSyncEngine({
      url: '',
      storage: new IndexedDBStorageAdapter('test-db'),
    });
    await engine.whenReady();
  });

  afterEach(async () => {
    resetSyncEngine();
    indexedDB.deleteDatabase('test-db');
  });

  it('initializes empty Automerge doc with Zustand state', async () => {
    const INITIAL_STATE = { counter: 42, message: 'Hello' };
    
    const useStore = create(
      sync(
        (set) => ({
          ...INITIAL_STATE,
          increment: () => set(s => ({ counter: s.counter + 1 })),
        }),
        { docId: 'test-doc' }
      )
    );

    await new Promise(resolve => setTimeout(resolve, 500));
    
    const docFromAutomerge = await readDoc<any>('test-doc');
    
    // Bug: Without fix, docFromAutomerge would be undefined/empty
    // Fix: With A.stats().numOps check, it contains Zustand state
    expect(docFromAutomerge.counter).toBe(42);
    expect(docFromAutomerge.message).toBe('Hello');
  });
});