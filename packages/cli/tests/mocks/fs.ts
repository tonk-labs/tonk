import { vi } from 'vitest';

export const mockFs = {
  readFile: vi.fn(),
  writeFile: vi.fn(),
  readdir: vi.fn(),
  mkdir: vi.fn(),
  pathExists: vi.fn(),
  copy: vi.fn(),
  remove: vi.fn(),
  ensureDir: vi.fn(),
  stat: vi.fn(),
  readJson: vi.fn(),
  writeJson: vi.fn(),
  outputFile: vi.fn(),
  outputJson: vi.fn(),
  exists: vi.fn(),
  createReadStream: vi.fn(),
  createWriteStream: vi.fn(),
};

export function setupFsMocks() {
  // Default mock implementations
  mockFs.pathExists.mockResolvedValue(false);
  mockFs.readdir.mockResolvedValue([]);
  mockFs.mkdir.mockResolvedValue(undefined);
  mockFs.copy.mockResolvedValue(undefined);
  mockFs.writeFile.mockResolvedValue(undefined);
  mockFs.readFile.mockResolvedValue('');
  mockFs.ensureDir.mockResolvedValue(undefined);
  mockFs.remove.mockResolvedValue(undefined);
  mockFs.readJson.mockResolvedValue({});
  mockFs.writeJson.mockResolvedValue(undefined);
  mockFs.outputFile.mockResolvedValue(undefined);
  mockFs.outputJson.mockResolvedValue(undefined);
  mockFs.exists.mockResolvedValue(false);
  mockFs.stat.mockResolvedValue({
    isDirectory: () => false,
    isFile: () => true,
  });

  return mockFs;
}
