import {Command} from 'commander';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs-extra';
import {fileURLToPath} from 'url';
import {
  trackCommand,
  trackCommandError,
  trackCommandSuccess,
  shutdownAnalytics,
} from '../utils/analytics.js';

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
    const startTime = Date.now();

    try {
      trackCommand('init', {
        hasDirectory: !!dirPath,
        targetDirectory: dirPath || 'current',
      });

      const cwd = process.cwd();
      const target = dirPath ? path.join(cwd, dirPath) : cwd;

      // Check if target already exists and has files
      const targetExists = await fs.pathExists(target);
      let existingFiles = 0;
      if (targetExists) {
        const files = await fs.readdir(target);
        existingFiles = files.filter(file => !file.startsWith('.')).length;
      }

      console.log(chalk.blue(`🚀 Initializing Tonk repository in ${target}`));

      const templatePath = await resolvePackagePath(`template`);

      // Create directory if needed
      let directoryCreated = false;
      if (dirPath && !targetExists) {
        await fs.mkdir(target);
        directoryCreated = true;
        console.log(chalk.green(`✅ Created directory: ${dirPath}`));
      }

      // Count files to be copied
      let copiedFiles = 0;
      let skippedFiles = 0;

      await fs.copy(templatePath, target, {
        filter: (src: string) => {
          // Get the relative path from the template directory
          const relativePath = path.relative(templatePath, src);
          // Only filter out node_modules and .git within the template
          const shouldCopy = !relativePath
            .split(path.sep)
            .some(part => part === 'node_modules' || part === '.git');

          if (!shouldCopy) {
            console.log(chalk.yellow(`Skipping: ${relativePath}`));
            skippedFiles++;
          } else if (relativePath) {
            copiedFiles++;
          }

          return shouldCopy;
        },
        overwrite: true,
        errorOnExist: false,
      });

      console.log(chalk.green(`✅ Successfully initialized Tonk repository!`));
      console.log(chalk.cyan(`📁 Files copied: ${copiedFiles}`));
      if (skippedFiles > 0) {
        console.log(chalk.yellow(`📁 Files skipped: ${skippedFiles}`));
      }

      const duration = Date.now() - startTime;
      trackCommandSuccess('init', duration, {
        hasDirectory: !!dirPath,
        directoryCreated,
        existingFiles,
        copiedFiles,
        skippedFiles,
        targetPath: target,
        templatePath,
      });

      await shutdownAnalytics();
    } catch (error) {
      const duration = Date.now() - startTime;

      // Determine error type
      let errorType = 'unknown_error';
      if (error instanceof Error) {
        if (error.message.includes('ENOENT')) {
          errorType = 'template_not_found';
        } else if (error.message.includes('EACCES')) {
          errorType = 'permission_denied';
        } else if (error.message.includes('EEXIST')) {
          errorType = 'directory_exists';
        } else if (error.message.includes('Could not locate')) {
          errorType = 'package_resolution_failed';
        }
      }

      trackCommandError('init', error as Error, duration, {
        hasDirectory: !!dirPath,
        errorType,
        targetPath: dirPath ? path.join(process.cwd(), dirPath) : process.cwd(),
      });

      console.error(
        chalk.red('❌ Failed to initialize Tonk repository:'),
        error,
      );
      await shutdownAnalytics();
      process.exit(1);
    }
  });
