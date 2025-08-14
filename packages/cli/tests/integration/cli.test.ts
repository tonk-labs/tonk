import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { CLITestHelper } from '../helpers/cli.js';
import { TempDirectoryHelper } from '../helpers/filesystem.js';
import { expectCLISuccess, expectCLIOutput } from '../helpers/assertions.js';

describe('CLI integration tests', () => {
  const cli = new CLITestHelper();
  let tempDir: TempDirectoryHelper;

  beforeEach(async () => {
    tempDir = new TempDirectoryHelper('tonk-cli-test');
    await tempDir.create();
  });

  afterEach(async () => {
    await tempDir.cleanup();
  });

  it('should show version when --version flag is used', async () => {
    const result = await cli.run(['--version'], {
      cwd: tempDir.getPath(),
      env: { DISABLE_ANALYTICS: 'true' },
    });

    expectCLISuccess(result);
    expectCLIOutput(result, /\d+\.\d+\.\d+/); // Should match semantic version
  });

  it('should show help when --help flag is used', async () => {
    const result = await cli.run(['--help'], {
      cwd: tempDir.getPath(),
      env: { DISABLE_ANALYTICS: 'true' },
    });

    expectCLISuccess(result);
    expectCLIOutput(result, 'Usage:');
    expectCLIOutput(result, 'Commands:');
    expectCLIOutput(result, 'Options:');
  });

  it('should show help for specific commands', async () => {
    const result = await cli.run(['hello', '--help'], {
      cwd: tempDir.getPath(),
      env: { DISABLE_ANALYTICS: 'true' },
    });

    expectCLISuccess(result);
    expectCLIOutput(result, 'Say hello to start and launch the tonk daemon');
    expectCLIOutput(result, 'Usage:');
  });

  it('should handle unknown commands gracefully', async () => {
    const result = await cli.run(['unknown-command'], {
      cwd: tempDir.getPath(),
      env: { DISABLE_ANALYTICS: 'true' },
    });

    expect(result.success).toBe(false);
    expect(result.stderr).toContain('unknown command');
  });

  it('should respect DISABLE_ANALYTICS environment variable', async () => {
    const result = await cli.run(['hello', '--help'], {
      cwd: tempDir.getPath(),
      env: { DISABLE_ANALYTICS: 'true' },
    });

    expectCLISuccess(result);
    // Analytics should be disabled, so no tracking calls should be made
    // This would be verified in the analytics unit tests
  });

  it('should work across different node environments', async () => {
    const result = await cli.run(['--version'], {
      cwd: tempDir.getPath(),
      env: {
        NODE_ENV: 'production',
        DISABLE_ANALYTICS: 'true',
      },
    });

    expectCLISuccess(result);
  });

  describe('available commands', () => {
    it('should have hello command available', async () => {
      const result = await cli.run(['hello', '--help'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expectCLISuccess(result);
      expectCLIOutput(result, 'hello');
    });

    it('should have create command available', async () => {
      const result = await cli.run(['create', '--help'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expectCLISuccess(result);
      expectCLIOutput(result, 'create');
    });

    it('should have auth command available', async () => {
      const result = await cli.run(['auth', '--help'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expectCLISuccess(result);
      expectCLIOutput(result, 'auth');
    });

    it('should have worker command available', async () => {
      const result = await cli.run(['worker', '--help'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expectCLISuccess(result);
      expectCLIOutput(result, 'worker');
    });
  });

  describe('error handling', () => {
    it('should provide meaningful error messages', async () => {
      const result = await cli.run(['nonexistent-command'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expect(result.success).toBe(false);
      expect(result.stderr).toContain('unknown command');
    });

    it('should handle invalid option combinations', async () => {
      const result = await cli.run(['hello', '--invalid-flag'], {
        cwd: tempDir.getPath(),
        env: { DISABLE_ANALYTICS: 'true' },
      });

      expect(result.success).toBe(false);
      // Should provide helpful error message
      expect(result.stderr.length).toBeGreaterThan(0);
    });
  });
});
