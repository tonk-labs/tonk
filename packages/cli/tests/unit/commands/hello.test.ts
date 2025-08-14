import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { promisify } from 'util';

const mockExecAsync = vi.fn();

// Mock dependencies - make exec return a function that can be promisified
vi.mock('child_process', () => ({
  exec: vi.fn((cmd: string, callback: any) => {
    // Simulate the callback style, but we'll control it via mockExecAsync
    const result = mockExecAsync(cmd);
    if (result && typeof result.then === 'function') {
      // Handle promise result
      result
        .then((res: any) => callback(null, res))
        .catch((err: any) => callback(err));
    } else if (result instanceof Error) {
      callback(result);
    } else {
      callback(null, result);
    }
  }),
}));

vi.mock('util', () => ({
  promisify: vi.fn(() => mockExecAsync),
}));

vi.mock('util', () => ({
  promisify: vi.fn(_fn => mockExecAsync),
}));

vi.mock('chalk', () => ({
  default: {
    blue: vi.fn(text => text),
    green: vi.fn(text => text),
    yellow: vi.fn(text => text),
    red: vi.fn(text => text),
    cyan: vi.fn(text => text),
    bold: {
      green: vi.fn(text => text),
    },
  },
}));

vi.mock('env-paths', () => ({
  default: vi.fn(() => ({ data: '/test/tonk/home' })),
}));

vi.mock('../../../src/commands/hello/index.js', () => ({
  default: vi.fn(),
}));

vi.mock('../../../src/utils/analytics.js', () => ({
  trackCommand: vi.fn(),
  trackCommandSuccess: vi.fn(),
  trackCommandError: vi.fn(),
  shutdownAnalytics: vi.fn().mockResolvedValue(undefined),
}));

// Import after mocks
const { helloCommand } = await import('../../../src/commands/hello.js');

describe('hello command', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(promisify).mockReturnValue(mockExecAsync);

    // Mock console methods
    vi.spyOn(console, 'log').mockImplementation(() => {});
    vi.spyOn(console, 'error').mockImplementation(() => {});

    // Reset process.exitCode
    process.exitCode = 0;
  });

  afterEach(() => {
    vi.clearAllMocks();
    process.exitCode = 0;
  });

  it('should have correct command configuration', () => {
    expect(helloCommand.name()).toBe('hello');
    expect(helloCommand.description()).toBe(
      'Say hello to start and launch the tonk daemon'
    );
  });

  it('should install pm2 when not present', async () => {
    // Mock pm2 not being installed initially
    mockExecAsync
      .mockRejectedValueOnce(new Error('pm2: command not found')) // pm2 --version fails
      .mockResolvedValueOnce({ stdout: '', stderr: '' }) // npm install -g pm2 succeeds
      .mockResolvedValueOnce({ stdout: 'no processes', stderr: '' }) // pm2 list
      .mockResolvedValueOnce({ stdout: '/usr/local/bin/tonk', stderr: '' }) // which tonk
      .mockResolvedValueOnce({ stdout: '#!/usr/bin/env node', stderr: '' }) // head -1 tonk
      .mockResolvedValueOnce({ stdout: '', stderr: '' }); // pm2 start

    // Execute the command
    await helloCommand.parseAsync([], { from: 'user' });

    expect(mockExecAsync).toHaveBeenCalledWith('pm2 --version');
    expect(mockExecAsync).toHaveBeenCalledWith('npm install -g pm2');
  });

  it('should restart existing tonk daemon when already running', async () => {
    // Mock pm2 installed and tonk already running
    mockExecAsync
      .mockResolvedValueOnce({ stdout: 'pm2@5.0.0', stderr: '' }) // pm2 --version
      .mockResolvedValueOnce({ stdout: 'tonkserver running', stderr: '' }) // pm2 list with tonkserver
      .mockResolvedValueOnce({ stdout: '', stderr: '' }); // pm2 restart tonkserver

    const { trackCommandSuccess } = await import(
      '../../../src/utils/analytics.js'
    );

    await helloCommand.parseAsync([], { from: 'user' });

    expect(mockExecAsync).toHaveBeenCalledWith('pm2 restart tonkserver');
    expect(trackCommandSuccess).toHaveBeenCalledWith(
      'hello',
      expect.any(Number),
      {
        pm2AlreadyInstalled: true,
        tonkAlreadyRunning: true,
      }
    );
  });

  it('should start new tonk daemon with bash script', async () => {
    // Mock pm2 installed, tonk not running, bash script
    mockExecAsync
      .mockResolvedValueOnce({ stdout: 'pm2@5.0.0', stderr: '' }) // pm2 --version
      .mockResolvedValueOnce({ stdout: 'no processes', stderr: '' }) // pm2 list
      .mockResolvedValueOnce({ stdout: '/usr/local/bin/tonk', stderr: '' }) // which tonk
      .mockResolvedValueOnce({ stdout: '#!/bin/bash', stderr: '' }) // head -1 tonk (bash script)
      .mockResolvedValueOnce({ stdout: '', stderr: '' }); // pm2 start bash

    await helloCommand.parseAsync([], { from: 'user' });

    expect(mockExecAsync).toHaveBeenCalledWith(
      'NODE_ENV=development pm2 start bash --name tonkserver -- /usr/local/bin/tonk -d'
    );
  });

  it('should start new tonk daemon with node script', async () => {
    // Mock pm2 installed, tonk not running, node script
    mockExecAsync
      .mockResolvedValueOnce({ stdout: 'pm2@5.0.0', stderr: '' }) // pm2 --version
      .mockResolvedValueOnce({ stdout: 'no processes', stderr: '' }) // pm2 list
      .mockResolvedValueOnce({ stdout: '/usr/local/bin/tonk', stderr: '' }) // which tonk
      .mockResolvedValueOnce({ stdout: '#!/usr/bin/env node', stderr: '' }) // head -1 tonk (node script)
      .mockResolvedValueOnce({ stdout: '', stderr: '' }); // pm2 start

    await helloCommand.parseAsync([], { from: 'user' });

    expect(mockExecAsync).toHaveBeenCalledWith(
      'NODE_ENV=development pm2 start /usr/local/bin/tonk --name tonkserver -- -d'
    );
  });

  it('should handle pm2 installation failure', async () => {
    mockExecAsync
      .mockRejectedValueOnce(new Error('pm2: command not found')) // pm2 --version fails
      .mockRejectedValueOnce(new Error('npm install failed')); // npm install -g pm2 fails

    const { trackCommandError } = await import(
      '../../../src/utils/analytics.js'
    );

    await helloCommand.parseAsync([], { from: 'user' });

    expect(trackCommandError).toHaveBeenCalledWith(
      'hello',
      expect.any(Error),
      expect.any(Number)
    );
    expect(process.exitCode).toBe(1);
  });

  it('should handle daemon start failure', async () => {
    mockExecAsync
      .mockResolvedValueOnce({ stdout: 'pm2@5.0.0', stderr: '' }) // pm2 --version
      .mockResolvedValueOnce({ stdout: 'no processes', stderr: '' }) // pm2 list
      .mockResolvedValueOnce({ stdout: '/usr/local/bin/tonk', stderr: '' }) // which tonk
      .mockResolvedValueOnce({ stdout: '#!/usr/bin/env node', stderr: '' }) // head -1 tonk
      .mockRejectedValueOnce(new Error('pm2 start failed')); // pm2 start fails

    const { trackCommandError } = await import(
      '../../../src/utils/analytics.js'
    );

    await helloCommand.parseAsync([], { from: 'user' });

    expect(trackCommandError).toHaveBeenCalledWith(
      'hello',
      expect.any(Error),
      expect.any(Number)
    );
    expect(process.exitCode).toBe(1);
  });

  it('should display welcome animation on success', async () => {
    mockExecAsync
      .mockResolvedValueOnce({ stdout: 'pm2@5.0.0', stderr: '' }) // pm2 --version
      .mockResolvedValueOnce({ stdout: 'no processes', stderr: '' }) // pm2 list
      .mockResolvedValueOnce({ stdout: '/usr/local/bin/tonk', stderr: '' }) // which tonk
      .mockResolvedValueOnce({ stdout: '#!/usr/bin/env node', stderr: '' }) // head -1 tonk
      .mockResolvedValueOnce({ stdout: '', stderr: '' }); // pm2 start

    const displayTonkAnimation = await import(
      '../../../src/commands/hello/index.js'
    );

    await helloCommand.parseAsync([], { from: 'user' });

    expect(displayTonkAnimation.default).toHaveBeenCalled();
    expect(console.log).toHaveBeenCalledWith(
      expect.stringContaining('🏠 Tonk Home:'),
      expect.stringContaining('/test/tonk/home')
    );
  });
});
