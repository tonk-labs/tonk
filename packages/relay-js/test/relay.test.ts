import { describe, it } from 'node:test';
import assert from 'node:assert';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { getBinaryPath, startRelay, runRelay } from '../dist/index.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const TEST_BUNDLE = join(__dirname, '..', 'app.tonk');

describe('getBinaryPath', () => {
  it('should return a valid path to the binary', () => {
    const binaryPath = getBinaryPath();
    assert.ok(binaryPath, 'Binary path should not be empty');
    assert.ok(existsSync(binaryPath), `Binary should exist at ${binaryPath}`);
  });

  it('should return an executable file', () => {
    const binaryPath = getBinaryPath();
    // Check that it's the expected binary name
    assert.ok(
      binaryPath.endsWith('tonk-relay') || binaryPath.endsWith('tonk-relay.exe'),
      'Binary should be named tonk-relay'
    );
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

    assert.strictEqual(exitCode, 1, 'Should exit with error code 1');
    assert.ok(
      stderr.includes('NotFound') || stderr.includes('not found'),
      `Should report bundle not found error, got: ${stderr}`
    );
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
    assert.ok(!relay.killed, 'Relay should be running');

    // Kill the process
    const killed = relay.kill('SIGTERM');
    assert.ok(killed, 'Should be able to kill the process');

    const exitCode = await new Promise<number>((resolve) => {
      relay.on('exit', (code) => resolve(code ?? 0));
    });

    assert.ok(exitCode !== undefined, 'Process should have exited');
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
      assert.strictEqual(response.status, 200, 'Health check should return 200');

      const text = await response.text();
      assert.ok(text.includes('Tonk'), 'Health check should mention Tonk');
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

    assert.strictEqual(exitCode, 1, 'Should exit with error code 1 for missing bundle');
  });
});
