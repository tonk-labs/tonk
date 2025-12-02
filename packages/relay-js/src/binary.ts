import { platform, arch } from 'node:os';
import { dirname, join } from 'node:path';
import { createRequire } from 'node:module';
import { chmodSync } from 'node:fs';

const require = createRequire(import.meta.url);

/**
 * Get the path to the tonk-relay binary for the current platform.
 * Ensures the binary is executable
 *
 * @returns The absolute path to the binary
 * @throws Error if the current platform is not supported
 */
export function getBinaryPath(): string {
  const platformKey = `${platform()}-${arch()}`;
  const packageName = `@tonk/relay-${platformKey}`;

  try {
    const packagePath = require.resolve(`${packageName}/package.json`);
    const packageDir = dirname(packagePath);
    const binaryName = platform() === 'win32' ? 'tonk-relay.exe' : 'tonk-relay';
    const binaryPath = join(packageDir, 'bin', binaryName);

    if (platform() !== 'win32') {
      chmodSync(binaryPath, 0o755);
    }

    return binaryPath;
  } catch {
    throw new Error(
      `Unsupported platform: ${platformKey}. ` +
        `The @tonk/relay package does not include a binary for your platform. ` +
        `Supported platforms: darwin-arm64, darwin-x64, linux-arm64, linux-x64, win32-arm64, win32-x64`
    );
  }
}
