import { vi } from 'vitest';

export const mockFetch = vi.fn();
export const mockNodeFetch = vi.fn();

export function setupNetworkMocks() {
  // Mock successful fetch responses
  mockFetch.mockResolvedValue({
    ok: true,
    status: 200,
    json: vi.fn().mockResolvedValue({}),
    text: vi.fn().mockResolvedValue(''),
    headers: new Headers(),
  });

  mockNodeFetch.mockResolvedValue({
    ok: true,
    status: 200,
    json: vi.fn().mockResolvedValue({}),
    text: vi.fn().mockResolvedValue(''),
    headers: new Headers(),
  });

  // Mock global fetch
  vi.stubGlobal('fetch', mockFetch);

  return { mockFetch, mockNodeFetch };
}
