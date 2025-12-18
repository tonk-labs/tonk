import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import 'fake-indexeddb/auto';
import { BundleStorage } from '../bundleStorage';

describe('BundleStorage', () => {
  let storage: BundleStorage;

  beforeEach(async () => {
    // Reset storage for each test
    storage = new BundleStorage();
    // Clear existing data
    const list = await storage.list();
    for (const b of list) {
      await storage.delete(b.id);
    }
  });

  afterEach(() => {
    // Clean up IndexedDB after each test if needed
  });

  it('should save and retrieve a bundle', async () => {
    const id = crypto.randomUUID();
    const data = {
      name: 'test-bundle',
      size: 1024,
      bytes: new Uint8Array([1, 2, 3]),
    };

    await storage.save(id, data);

    const retrieved = await storage.get(id);
    expect(retrieved).not.toBeNull();
    expect(retrieved?.id).toBe(id);
    expect(retrieved?.name).toBe(data.name);
    expect(retrieved?.bytes).toEqual(data.bytes);
  });

  it('should return null for non-existent bundle', async () => {
    const retrieved = await storage.get('non-existent-id');
    expect(retrieved).toBeNull();
  });

  it('should list all bundles without loading bytes', async () => {
    const id1 = crypto.randomUUID();
    const id2 = crypto.randomUUID();

    await storage.save(id1, {
      name: 'b1',
      size: 100,
      bytes: new Uint8Array([1]),
    });
    await storage.save(id2, {
      name: 'b2',
      size: 200,
      bytes: new Uint8Array([2]),
    });

    const list = await storage.list();
    expect(list).toHaveLength(2);

    const b1 = list.find(b => b.id === id1);
    expect(b1).toBeDefined();
    expect(b1?.name).toBe('b1');
    // @ts-expect-error - 'bytes' should not be present in list view (runtime check)
    expect(b1?.bytes).toBeUndefined();
  });

  it('should delete a bundle', async () => {
    const id = crypto.randomUUID();
    await storage.save(id, {
      name: 'to-delete',
      size: 50,
      bytes: new Uint8Array([]),
    });

    await storage.delete(id);

    const retrieved = await storage.get(id);
    expect(retrieved).toBeNull();
  });
});
