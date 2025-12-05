import { describe, it, expect } from 'bun:test';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { getBinaryPath, startRelay, runRelay } from '../dist/index.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const TEST_BUNDLE = join(__dirname, '..', 'app.tonk');

describe('getBinaryPath', () => {
  it('should return a valid path to the binary', () => {
    const binaryPath = getBinaryPath();
    expect(binaryPath).toBeTruthy();
    expect(existsSync(binaryPath)).toBeTruthy();
  });

  it('should return an executable file', () => {
    const binaryPath = getBinaryPath();
    // Check that it's the expected binary name
    expect(
      binaryPath.endsWith('tonk-relay') || binaryPath.endsWith('tonk-relay.exe')
    ).toBeTruthy();
  });
});

describe('startRelay', () => {
  it('should fail with missing bundle path error', async () => {
    const relay = startRelay({
      port: 19999,
      bundlePath: '/nonexistent/path/bundle.tonk',
      stdio: 'pipe',
    });

    // Collect stderr
    let stderr = '';
    relay.stderr?.on('data', (chunk) => {
      stderr += chunk.toString();
    });

    const exitCode = await new Promise<number>((resolve) => {
      relay.on('exit', (code) => resolve(code ?? 1));
    });

    expect(exitCode).toBe(1);
    expect(
      stderr.includes('NotFound') || stderr.includes('not found')
    ).toBeTruthy();
  });

  it('should start with valid bundle and be killable', async () => {
    const relay = startRelay({
      port: 19998,
      bundlePath: TEST_BUNDLE,
      storagePath: '/tmp/relay-test-storage',
      stdio: 'pipe',
    });

    // Give the server time to start
    await new Promise((r) => setTimeout(r, 500));

    // Verify the process is running
    expect(relay.killed).toBeFalsy();

    // Kill the process
    const killed = relay.kill('SIGTERM');
    expect(killed).toBeTruthy();

    const exitCode = await new Promise<number>((resolve) => {
      relay.on('exit', (code) => resolve(code ?? 0));
    });

    expect(exitCode).toBeDefined();
  });

  it('should respond to health check', async () => {
    const port = 19997;
    const relay = startRelay({
      port,
      bundlePath: TEST_BUNDLE,
      storagePath: '/tmp/relay-test-storage-2',
      stdio: 'pipe',
    });

    try {
      // Wait for server to start
      await new Promise((r) => setTimeout(r, 1000));

      // Make health check request
      const response = await fetch(`http://127.0.0.1:${port}/`);
      expect(response.status).toBe(200);

      const text = await response.text();
      expect(text.includes('Tonk')).toBeTruthy();
    } finally {
      relay.kill('SIGTERM');
      await new Promise<void>((resolve) => {
        relay.on('exit', () => resolve());
      });
    }
  });
});

describe('runRelay', () => {
  it('should return a promise that resolves with exit code', async () => {
    const exitCode = await runRelay({
      port: 19996,
      bundlePath: '/nonexistent/bundle.tonk',
      stdio: 'pipe',
    });

    expect(exitCode).toBe(1);
  });
});
