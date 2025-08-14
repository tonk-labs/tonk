import fs from 'fs-extra';
import path from 'path';
import os from 'os';

export class TempDirectoryHelper {
  private tempDir: string;

  constructor(prefix = 'tonk-test') {
    this.tempDir = path.join(
      os.tmpdir(),
      `${prefix}-${Date.now()}-${Math.random().toString(36).substring(2, 11)}`
    );
  }

  async create(): Promise<string> {
    await fs.ensureDir(this.tempDir);
    return this.tempDir;
  }

  async cleanup(): Promise<void> {
    if (await fs.pathExists(this.tempDir)) {
      await fs.remove(this.tempDir);
    }
  }

  getPath(...pathSegments: string[]): string {
    return path.join(this.tempDir, ...pathSegments);
  }

  async writeFile(relativePath: string, content: string): Promise<void> {
    const fullPath = this.getPath(relativePath);
    await fs.ensureDir(path.dirname(fullPath));
    await fs.writeFile(fullPath, content);
  }

  async writeJson(relativePath: string, data: any): Promise<void> {
    const fullPath = this.getPath(relativePath);
    await fs.ensureDir(path.dirname(fullPath));
    await fs.writeJson(fullPath, data, { spaces: 2 });
  }

  async readFile(relativePath: string): Promise<string> {
    return fs.readFile(this.getPath(relativePath), 'utf-8');
  }

  async readJson(relativePath: string): Promise<any> {
    return fs.readJson(this.getPath(relativePath));
  }

  async exists(relativePath: string): Promise<boolean> {
    return fs.pathExists(this.getPath(relativePath));
  }

  async createDirectory(relativePath: string): Promise<void> {
    await fs.ensureDir(this.getPath(relativePath));
  }
}

export function createTempDir(prefix?: string): TempDirectoryHelper {
  return new TempDirectoryHelper(prefix);
}
