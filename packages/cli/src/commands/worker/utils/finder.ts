import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';

/**
 * Find the worker root directory by looking for package.json and worker.config.js
 * Searches in the given directory and parent directories
 */
export async function findWorkerRoot(startDir: string): Promise<string | null> {
  try {
    let currentDir = path.resolve(startDir);
    const rootDir = path.parse(currentDir).root;

    // Search up to the root directory
    while (currentDir !== rootDir) {
      const packageJsonPath = path.join(currentDir, 'package.json');
      const workerConfigPath = path.join(currentDir, 'worker.config.js');

      // Check if both files exist
      if (fs.existsSync(packageJsonPath) && fs.existsSync(workerConfigPath)) {
        return currentDir;
      }

      // Move up one directory
      const parentDir = path.dirname(currentDir);

      // If we've reached the root, stop searching
      if (parentDir === currentDir) {
        break;
      }

      currentDir = parentDir;
    }

    return null;
  } catch (error) {
    console.error('Error finding worker root:', error);
    return null;
  }
}

/**
 * Find an available port starting from the given port number
 * @param startPort The port to start checking from
 * @returns A promise that resolves to an available port
 */
export async function findAvailablePort(startPort: number): Promise<number> {
  let port = startPort;

  const isPortAvailable = (port: number): Promise<boolean> => {
    return new Promise(resolve => {
      const server = net.createServer();

      server.once('error', () => {
        resolve(false);
      });

      server.once('listening', () => {
        server.close();
        resolve(true);
      });

      server.listen(port);
    });
  };

  while (!(await isPortAvailable(port))) {
    port++;
  }

  return port;
}
