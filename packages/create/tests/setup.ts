import { vi, beforeEach, afterAll } from 'vitest';

// Mock fs-extra
vi.mock('fs-extra', () => ({
  default: {
    pathExists: vi.fn().mockResolvedValue(true),
    ensureDir: vi.fn().mockResolvedValue(undefined),
    copy: vi.fn().mockResolvedValue(undefined),
    readJson: vi.fn().mockResolvedValue({}),
    writeJson: vi.fn().mockResolvedValue(undefined),
    writeJSON: vi.fn().mockResolvedValue(undefined),
    readdir: vi.fn().mockResolvedValue([]),
    readFileSync: vi
      .fn()
      .mockReturnValue('{"name": "test", "version": "1.0.0"}'),
  },
  pathExists: vi.fn().mockResolvedValue(true),
  ensureDir: vi.fn().mockResolvedValue(undefined),
  copy: vi.fn().mockResolvedValue(undefined),
  readJson: vi.fn().mockResolvedValue({}),
  writeJson: vi.fn().mockResolvedValue(undefined),
  writeJSON: vi.fn().mockResolvedValue(undefined),
  readdir: vi.fn().mockResolvedValue([]),
  readFileSync: vi.fn().mockReturnValue('{"name": "test", "version": "1.0.0"}'),
}));

// Mock child_process
vi.mock('child_process', () => ({
  execSync: vi.fn(() => '/usr/local/lib/node_modules'),
}));

// Mock inquirer
vi.mock('inquirer', () => ({
  default: {
    prompt: vi.fn(),
  },
}));

// Mock ora spinner
vi.mock('ora', () => ({
  default: vi.fn(() => ({
    start: vi.fn().mockReturnThis(),
    stop: vi.fn().mockReturnThis(),
    succeed: vi.fn().mockReturnThis(),
    fail: vi.fn().mockReturnThis(),
    text: '',
  })),
}));

// Mock process methods
const originalExit = process.exit;
const originalChdir = process.chdir;

beforeEach(() => {
  // Mock process.exit to prevent tests from actually exiting
  process.exit = vi.fn() as any;

  // Mock process.chdir
  process.chdir = vi.fn();

  // Clear all mocks
  vi.clearAllMocks();
});

afterAll(() => {
  // Restore original process methods
  process.exit = originalExit;
  process.chdir = originalChdir;
});
