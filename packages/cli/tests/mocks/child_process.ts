import { vi } from 'vitest';

export const mockChildProcess = {
  exec: vi.fn(),
  execSync: vi.fn(),
  spawn: vi.fn(),
  fork: vi.fn(),
};

export function setupChildProcessMocks() {
  // Default mock implementations
  mockChildProcess.execSync.mockReturnValue(
    Buffer.from('/usr/local/lib/node_modules')
  );

  mockChildProcess.exec.mockImplementation((_, callback) => {
    // Default success response
    if (callback) callback(null, 'mock output', '');
  });

  mockChildProcess.spawn.mockImplementation(() => ({
    stdout: {
      on: vi.fn((event, handler) => {
        if (event === 'data') handler(Buffer.from('mock stdout'));
      }),
    },
    stderr: {
      on: vi.fn((event, handler) => {
        if (event === 'data') handler(Buffer.from(''));
      }),
    },
    on: vi.fn((event, handler) => {
      if (event === 'close') handler(0);
    }),
    stdin: {
      write: vi.fn(),
      end: vi.fn(),
    },
  }));

  return mockChildProcess;
}
