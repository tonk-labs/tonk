import { describe, it, expect, vi, beforeEach } from 'vitest';
import { BundleManager } from '../bundleManager';
import { bundleStorage } from '../bundleStorage';
import { Bundle, initializeTonk } from '@tonk/core/slim';

// Mock bundleStorage
vi.mock('../bundleStorage', () => ({
  bundleStorage: {
    save: vi.fn(),
    get: vi.fn(),
    list: vi.fn(),
    delete: vi.fn(),
  },
}));

// Mock TonkCore Bundle and initializeTonk
vi.mock('@tonk/core/slim', () => ({
  Bundle: {
    fromBytes: vi.fn(),
  },
  initializeTonk: vi.fn().mockResolvedValue(undefined),
}));

describe('BundleManager', () => {
  let manager: BundleManager;

  beforeEach(() => {
    vi.clearAllMocks();
    manager = new BundleManager();
  });

  it('should load a bundle from file and save it', async () => {
    const file = new File(['dummy content'], 'app.tonk', { type: 'application/octet-stream' });

    const mockManifest = {
      entrypoints: ['my-app'],
    };

    const mockBundle = {
      getManifest: vi.fn().mockResolvedValue(mockManifest),
      free: vi.fn(),
    };

    // biome-ignore lint/suspicious/noExplicitAny: mocking requires any
    vi.mocked(Bundle.fromBytes).mockResolvedValue(mockBundle as any);
    vi.mocked(bundleStorage.save).mockResolvedValue(undefined);

    const bundleId = await manager.loadBundleFromFile(file);

    expect(bundleId).toBeDefined();
    expect(typeof bundleId).toBe('string');

    // Verify initialization
    expect(initializeTonk).toHaveBeenCalled();

    // Verify Bundle.fromBytes called
    expect(Bundle.fromBytes).toHaveBeenCalled();
    expect(mockBundle.free).toHaveBeenCalled();

    // Verify storage save called
    expect(bundleStorage.save).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        name: 'app.tonk',
        size: file.size,
        bytes: expect.any(Uint8Array),
      })
    );
  });

  it('should throw error if bundle is invalid (no manifest)', async () => {
    const file = new File(['bad'], 'bad.tonk');

    vi.mocked(Bundle.fromBytes).mockRejectedValue(new Error('Corrupt data'));

    await expect(manager.loadBundleFromFile(file)).rejects.toThrow('Invalid bundle: Corrupt data');
    expect(bundleStorage.save).not.toHaveBeenCalled();
  });

  it('should throw error if bundle has no entrypoints', async () => {
    const file = new File(['empty'], 'empty.tonk');

    const mockBundle = {
      getManifest: vi.fn().mockResolvedValue({ entrypoints: [] }),
      free: vi.fn(),
    };
    // biome-ignore lint/suspicious/noExplicitAny: mocking requires any
    vi.mocked(Bundle.fromBytes).mockResolvedValue(mockBundle as any);

    await expect(manager.loadBundleFromFile(file)).rejects.toThrow('Bundle has no entrypoints');
    expect(mockBundle.free).toHaveBeenCalled();
    expect(bundleStorage.save).not.toHaveBeenCalled();
  });

  it('should list bundles from storage', async () => {
    const mockList = [{ id: '1', name: 'b1' }];
    // biome-ignore lint/suspicious/noExplicitAny: mocking requires any
    vi.mocked(bundleStorage.list).mockResolvedValue(mockList as any);

    const result = await manager.listBundles();
    expect(result).toEqual(mockList);
    expect(bundleStorage.list).toHaveBeenCalled();
  });

  it('should delete bundle from storage', async () => {
    const id = '123';
    vi.mocked(bundleStorage.delete).mockResolvedValue(undefined);

    await manager.deleteBundle(id);
    expect(bundleStorage.delete).toHaveBeenCalledWith(id);
  });
});
