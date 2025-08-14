import { describe, it, expect, vi, beforeEach } from 'vitest';

// Mock dependencies
vi.mock('../../../src/config/environment.js', () => ({
  getDeploymentServiceUrl: vi.fn(() => 'https://api.example.com'),
}));

vi.mock('../../../src/lib/tonkAuth.js', () => ({
  tonkAuth: {
    getAuthToken: vi.fn(),
  },
}));

// Mock fetch globally
global.fetch = vi.fn();

describe('server utils', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('fetchUserServers', () => {
    it('should fetch user servers successfully', async () => {
      // Setup mocks
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue('test-token');

      vi.mocked(global.fetch).mockResolvedValue({
        ok: true,
        json: vi.fn().mockResolvedValue({
          success: true,
          servers: ['server1', 'server2', 'server3'],
        }),
      } as any);

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );
      const servers = await fetchUserServers();

      expect(servers).toEqual(['server1', 'server2', 'server3']);
      expect(global.fetch).toHaveBeenCalledWith(
        'https://api.example.com/list-user-servers',
        {
          method: 'GET',
          headers: {
            Authorization: 'Bearer test-token',
          },
        }
      );
    });

    it('should return empty array when no servers found', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue('test-token');

      vi.mocked(global.fetch).mockResolvedValue({
        ok: true,
        json: vi.fn().mockResolvedValue({
          success: true,
          servers: [],
        }),
      } as any);

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );
      const servers = await fetchUserServers();

      expect(servers).toEqual([]);
    });

    it('should handle missing servers field in response', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue('test-token');

      vi.mocked(global.fetch).mockResolvedValue({
        ok: true,
        json: vi.fn().mockResolvedValue({
          success: true,
          // no servers field
        }),
      } as any);

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );
      const servers = await fetchUserServers();

      expect(servers).toEqual([]);
    });

    it('should throw error when no auth token', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue(null);

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );

      await expect(fetchUserServers()).rejects.toThrow(
        'Failed to get authentication token'
      );
    });

    it('should handle API error responses', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue('test-token');

      vi.mocked(global.fetch).mockResolvedValue({
        ok: false,
        json: vi.fn().mockResolvedValue({
          success: false,
          error: 'Unauthorized',
        }),
      } as any);

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );

      await expect(fetchUserServers()).rejects.toThrow(
        'Failed to fetch servers: Unauthorized'
      );
    });

    it('should handle network errors', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockResolvedValue('test-token');

      vi.mocked(global.fetch).mockRejectedValue(new Error('Network error'));

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );

      await expect(fetchUserServers()).rejects.toThrow(
        'Failed to fetch servers: Network error'
      );
    });

    it('should handle auth token retrieval errors', async () => {
      const { tonkAuth } = await import('../../../src/lib/tonkAuth.js');
      vi.mocked(tonkAuth.getAuthToken).mockRejectedValue(
        new Error('Auth error')
      );

      const { fetchUserServers } = await import(
        '../../../src/utils/serverUtils.js'
      );

      await expect(fetchUserServers()).rejects.toThrow(
        'Failed to fetch servers: Auth error'
      );
    });
  });
});
