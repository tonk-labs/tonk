import path from 'node:path';
import fs from 'node:fs';
import envPaths from 'env-paths';

import type {DocumentId} from '@automerge/automerge-repo';

export class RootNode {
  private configPath: string;

  constructor(configPath: string = '') {
    if (configPath === '') {
      this.configPath = path.join(envPaths('tonk').data, 'root.json');
    } else {
      this.configPath = configPath;
    }
  }

  getRootIdFilePath(): string {
    return this.configPath;
  }

  async getRootId(): Promise<DocumentId | undefined> {
    try {
      // Attempt to read the rootId from file
      const content = await fs.promises.readFile(
        this.getRootIdFilePath(),
        'utf8',
      );
      const data = JSON.parse(content);
      return data.rootId as DocumentId;
    } catch (error) {
      // If file doesn't exist or is invalid, generate a new rootId
      return undefined;
    }
  }

  async setRootId(rootId: DocumentId): Promise<void> {
    // Write rootId to file atomically
    const content = JSON.stringify({rootId: rootId, timestamp: Date.now()});
    const filePath = this.getRootIdFilePath();
    const tmpPath = `${filePath}.tmp`;

    try {
      // Ensure directory exists
      await fs.promises
        .mkdir(path.dirname(filePath), {recursive: true})
        .catch(() => {});

      // Write to temp file first
      await fs.promises.writeFile(tmpPath, content, 'utf8');
      // Rename is atomic on most filesystems
      await fs.promises.rename(tmpPath, filePath);
    } catch (error) {
      console.error('Failed to save rootId:', error);
      // Clean up tmp file if it exists
      try {
        await fs.promises.unlink(tmpPath).catch(() => {});
      } catch {
        // Ignore errors during cleanup
      }
    }
  }
}
