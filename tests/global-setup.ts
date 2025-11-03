import { execSync } from 'child_process';
import * as path from 'path';
import { fileURLToPath } from 'url';
import * as fs from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

export default async function globalSetup() {
  console.log('Setting up relay binary for tests...');

  let binaryPath: string;

  // 1. Check access to the proprietary relay
  if (process.env.TONK_RELAY_BINARY) {
    binaryPath = process.env.TONK_RELAY_BINARY;
    if (fs.existsSync(binaryPath)) {
      console.log(`Using proprietary relay from: ${binaryPath}`);
      fs.writeFileSync(
        path.join(__dirname, '.relay-binary'),
        binaryPath,
        'utf-8'
      );
      return;
    } else {
      console.warn(
        `TONK_RELAY_BINARY set to ${binaryPath} but file not found. Falling back to basic relay.`
      );
    }
  }

  // 2. Otherwise, build and use basic relay
  console.log('Building basic relay binary...');
  const relayPath = path.resolve(__dirname, '../packages/relay');
  const targetDir = path.join(relayPath, 'target', 'debug');
  const binaryName =
    process.platform === 'win32' ? 'tonk-relay.exe' : 'tonk-relay';
  binaryPath = path.join(targetDir, binaryName);

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
    console.log(`Basic relay binary built successfully at: ${binaryPath}`);

    fs.writeFileSync(
      path.join(__dirname, '.relay-binary'),
      binaryPath,
      'utf-8'
    );
  } catch (error) {
    console.error('Failed to build relay binary:', error);
    throw error;
  }
}
