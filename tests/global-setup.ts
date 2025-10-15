import { execSync } from 'child_process';
import * as path from 'path';
import { fileURLToPath } from 'url';
import * as fs from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export default async function globalSetup() {
  console.log('Building relay-rust binary for tests...');

  const relayPath = path.resolve(__dirname, '../packages/relay-rust');
  const targetDir = path.join(relayPath, 'target', 'debug');
  const binaryName =
    process.platform === 'win32' ? 'tonk-relay.exe' : 'tonk-relay';
  const binaryPath = path.join(targetDir, binaryName);

  try {
    execSync('cargo build', {
      cwd: relayPath,
      stdio: 'inherit',
      env: {
        ...process.env,
      },
    });

    if (!fs.existsSync(binaryPath)) {
      throw new Error(`Binary not found at ${binaryPath} after build`);
    }

    process.env.RELAY_BINARY_PATH = binaryPath;
    console.log(`Relay binary built successfully at: ${binaryPath}`);

    fs.writeFileSync(
      path.join(__dirname, '.relay-binary'),
      binaryPath,
      'utf-8'
    );
  } catch (error) {
    console.error('Failed to build relay-rust binary:', error);
    throw error;
  }
}
