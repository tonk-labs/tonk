import { vi } from 'vitest';

export const createMockFs = () => {
  const mockFileSystem = new Map<string, any>();

  return {
    pathExists: vi.fn((path: string) =>
      Promise.resolve(mockFileSystem.has(path))
    ),
    ensureDir: vi.fn((path: string) => {
      mockFileSystem.set(path, { type: 'directory' });
      return Promise.resolve();
    }),
    copy: vi.fn((src: string, dest: string) => {
      mockFileSystem.set(dest, mockFileSystem.get(src) || { type: 'file' });
      return Promise.resolve();
    }),
    readJson: vi.fn((path: string) => {
      const content = mockFileSystem.get(path);
      if (!content) return Promise.reject(new Error(`File not found: ${path}`));
      return Promise.resolve(
        content.json || { name: 'mock-package', version: '1.0.0' }
      );
    }),
    writeJson: vi.fn((path: string, data: any) => {
      mockFileSystem.set(path, { type: 'file', json: data });
      return Promise.resolve();
    }),
    writeJSON: vi.fn((path: string, data: any) => {
      mockFileSystem.set(path, { type: 'file', json: data });
      return Promise.resolve();
    }),
    readdir: vi.fn((path: string) => {
      if (mockFileSystem.has(path)) {
        return Promise.resolve(['file1.js', 'file2.js', 'package.json']);
      }
      return Promise.resolve([]);
    }),
    readFileSync: vi.fn((path: string) => {
      const content = mockFileSystem.get(path);
      if (!content) throw new Error(`File not found: ${path}`);
      return JSON.stringify(
        content.json || { name: 'mock-package', version: '1.0.0' }
      );
    }),

    // Test utilities
    setMockFile: (path: string, content: any) => {
      mockFileSystem.set(path, { type: 'file', json: content });
    },
    setMockDirectory: (path: string) => {
      mockFileSystem.set(path, { type: 'directory' });
    },
    getMockFileSystem: () => mockFileSystem,
    clearMockFileSystem: () => mockFileSystem.clear(),
  };
};
