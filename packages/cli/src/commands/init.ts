import {Command} from 'commander';
import path from 'path';
import {ChildProcess, spawn} from 'child_process';
import chalk from 'chalk';
import fs from 'fs-extra';
import {fileURLToPath} from 'url';
import {dir} from 'console';

/**
 * Resolves a package path by checking both local development and global installation paths
 * @param relativePath Path relative to the package root
 * @returns Resolved absolute path to the requested file/directory
 */
async function resolvePackagePath(relativePath: string): Promise<string> {
  try {
    // For ESM, get the directory name using import.meta.url
    const moduleUrl = import.meta.url;
    const moduleDirPath = path.dirname(fileURLToPath(moduleUrl));

    // Try local development path first
    const localPath = path.resolve(moduleDirPath, '..', relativePath);

    if (await fs.pathExists(localPath)) {
      return localPath;
    } else {
      // If local path doesn't exist, try global node_modules
      const {execSync} = await import('child_process');
      const globalNodeModules = execSync('npm root -g').toString().trim();

      // Look for the package in global node_modules
      const globalPath = path.join(
        globalNodeModules,
        '@tonk/create',
        relativePath,
      );

      if (await fs.pathExists(globalPath)) {
        return globalPath;
      } else {
        throw new Error(
          `Could not locate ${relativePath} in local or global paths`,
        );
      }
    }
  } catch (error) {
    console.error(`Error resolving path ${relativePath}:`, error);
    throw error;
  }
}

export const initCommand = new Command('init')
  .description('Create a new Tonk repo')
  .argument(
    '[<directory>]',
    'optionally creates a directory and initializes the tonk repo in that directory',
  )
  .action(async dirPath => {
    const cwd = process.cwd();
    const templatePath = await resolvePackagePath(`template`);
    const target = dirPath ? path.join(cwd, dirPath) : cwd;

    if (dirPath) {
      await fs.mkdir(target);
    }

    await fs.copy(templatePath, target, {
      filter: (src: string) => {
        // Get the relative path from the template directory
        const relativePath = path.relative(templatePath, src);
        // Only filter out node_modules and .git within the template
        const shouldCopy = !relativePath
          .split(path.sep)
          .some(part => part === 'node_modules' || part === '.git');
        if (!shouldCopy) {
          console.log(`Skipping: ${relativePath}`);
        }
        return shouldCopy;
      },
      overwrite: true,
      errorOnExist: false,
    });
  });
