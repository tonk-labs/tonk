#!/usr/bin/env tsx

/**
 * Bundle a Tonk app into a .tonk file
 *
 * This script:
 * 1. Builds the Tonk app if needed
 * 2. Bundles the dist/ folder contents according to @tonk/spec
 * 3. Includes tonk.config.json and proper metadata
 * 4. Outputs a .tonk bundle file
 */

import * as fs from 'fs';
import * as path from 'path';
import { execSync } from 'child_process';
import { ZipBundle } from '../packages/spec/dist/index.js';
import type { BundleFile } from '../packages/spec/dist/index.js';

// Colors for console output
const colors = {
  blue: '\x1b[34m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
  gray: '\x1b[90m',
  reset: '\x1b[0m',
};

function log(message: string, color = colors.reset) {
  console.log(`${color}${message}${colors.reset}`);
}

interface TonkConfig {
  name?: string;
  description?: string;
  plan?: {
    projectName?: string;
    description?: string;
  };
}

/**
 * Get MIME type for a file based on its extension
 */
function getMimeType(filePath: string): string {
  const ext = path.extname(filePath).toLowerCase();
  const mimeTypes: Record<string, string> = {
    // Web files
    '.html': 'text/html',
    '.css': 'text/css',
    '.js': 'application/javascript',
    '.mjs': 'application/javascript',
    '.json': 'application/json',

    // Images
    '.png': 'image/png',
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.gif': 'image/gif',
    '.svg': 'image/svg+xml',
    '.webp': 'image/webp',
    '.ico': 'image/x-icon',

    // Fonts
    '.woff': 'font/woff',
    '.woff2': 'font/woff2',
    '.ttf': 'font/ttf',
    '.otf': 'font/otf',

    // WebAssembly
    '.wasm': 'application/wasm',

    // Manifest
    '.webmanifest': 'application/json',

    // Text files
    '.txt': 'text/plain',
    '.md': 'text/markdown',
  };

  return mimeTypes[ext] || 'application/octet-stream';
}

/**
 * Recursively collect all files from a directory
 */
function collectFiles(
  dir: string,
  basePath = ''
): Array<{ virtualPath: string; realPath: string }> {
  const files: Array<{ virtualPath: string; realPath: string }> = [];

  const entries = fs.readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    const virtualPath = path.posix.join(basePath, entry.name);

    if (entry.isDirectory()) {
      // Recursively collect files from subdirectories
      files.push(...collectFiles(fullPath, virtualPath));
    } else {
      // Add file
      files.push({
        virtualPath: '/' + virtualPath,
        realPath: fullPath,
      });
    }
  }

  return files;
}

/**
 * Build the Tonk app
 */
async function buildApp(projectPath: string): Promise<void> {
  log('Building Tonk app...', colors.blue);

  try {
    // Check if package.json exists
    const packageJsonPath = path.join(projectPath, 'package.json');
    if (!fs.existsSync(packageJsonPath)) {
      throw new Error('package.json not found in project directory');
    }

    // Run build command
    log('  Running npm run build...', colors.gray);
    execSync('npm run build', {
      cwd: projectPath,
      stdio: 'pipe', // Capture output to avoid clutter
    });

    // Check if dist folder was created
    const distPath = path.join(projectPath, 'dist');
    if (!fs.existsSync(distPath)) {
      throw new Error('Build completed but dist folder not found');
    }

    log('Build completed successfully', colors.green);
  } catch (error) {
    log(`Build failed: ${error}`, colors.red);
    throw error;
  }
}

/**
 * Read tonk.config.json if it exists
 */
function readTonkConfig(projectPath: string): TonkConfig | null {
  const configPath = path.join(projectPath, 'tonk.config.json');
  if (fs.existsSync(configPath)) {
    try {
      return JSON.parse(fs.readFileSync(configPath, 'utf8'));
    } catch (error) {
      log(`Warning: Could not parse tonk.config.json: ${error}`, colors.yellow);
    }
  }
  return null;
}

/**
 * Main bundling function
 */
async function bundleTonkApp(appPath?: string): Promise<void> {
  // Get app path from argument or current directory
  const projectPath = appPath ? path.resolve(appPath) : process.cwd();

  // Read config to get app name
  const tonkConfig = readTonkConfig(projectPath);
  const appName =
    tonkConfig?.name ||
    tonkConfig?.plan?.projectName ||
    path.basename(projectPath);

  const outputPath = path.join(projectPath, `${appName}.tonk`);

  log('Starting Tonk app bundling process...', colors.blue);
  log(`  Project: ${projectPath}`, colors.gray);
  log(`  Output: ${outputPath}`, colors.gray);

  // Check if project exists
  if (!fs.existsSync(projectPath)) {
    log(`Project not found at ${projectPath}`, colors.red);
    process.exit(1);
  }

  try {
    // Step 1: Build the app
    await buildApp(projectPath);

    // Step 2: Read configuration
    const description =
      tonkConfig?.description ||
      tonkConfig?.plan?.description ||
      `${appName} Tonk application`;

    log(`Bundle metadata:`, colors.blue);
    log(`  Name: ${appName}`, colors.gray);
    log(`  Description: ${description}`, colors.gray);

    // Step 3: Create bundle
    const bundle = await ZipBundle.createEmpty({ version: 1 });

    // Set bundle metadata
    bundle.manifest.name = appName;
    bundle.manifest.description = description;
    bundle.manifest.createdAt = new Date().toISOString();
    bundle.manifest.metadata = {
      source: appName,
      tonkConfig: tonkConfig,
    };

    // Step 4: Collect files from dist directory
    const distPath = path.join(projectPath, 'dist');
    const distFiles = collectFiles(distPath);
    log(`Found ${distFiles.length} files in dist/`, colors.blue);

    // Step 5: Add dist files to bundle
    let addedCount = 0;
    for (const file of distFiles) {
      const fileData = fs.readFileSync(file.realPath);
      const arrayBuffer = fileData.buffer.slice(
        fileData.byteOffset,
        fileData.byteOffset + fileData.byteLength
      );

      const bundleFile: Omit<BundleFile, 'length'> = {
        path: file.virtualPath,
        contentType: getMimeType(file.realPath),
        compressed: true,
        lastModified: fs.statSync(file.realPath).mtime.toISOString(),
      };

      await bundle.addFile(bundleFile, arrayBuffer);
      addedCount++;

      // Show progress
      if (addedCount % 5 === 0 || file.virtualPath === '/index.html') {
        log(`    Added: ${file.virtualPath}`, colors.gray);
      }
    }

    // Step 6: Add tonk.config.json if it exists
    const configPath = path.join(projectPath, 'tonk.config.json');
    if (fs.existsSync(configPath)) {
      const configData = fs.readFileSync(configPath);
      const configArrayBuffer = configData.buffer.slice(
        configData.byteOffset,
        configData.byteOffset + configData.byteLength
      );

      const configFile: Omit<BundleFile, 'length'> = {
        path: '/tonk.config.json',
        contentType: 'application/json',
        compressed: true,
        lastModified: fs.statSync(configPath).mtime.toISOString(),
      };

      await bundle.addFile(configFile, configArrayBuffer);
      log(`    Added: /tonk.config.json`, colors.gray);
      addedCount++;
    }

    // Step 7: Set entrypoints
    const entrypoints: Record<string, string> = {};

    // Main entrypoint (required for web apps)
    if (bundle.hasFile('/index.html')) {
      entrypoints.main = '/index.html';
      log(`  Set entrypoint: main -> /index.html`, colors.gray);
    }

    // Config entrypoint
    if (bundle.hasFile('/tonk.config.json')) {
      entrypoints.config = '/tonk.config.json';
      log(`  Set entrypoint: config -> /tonk.config.json`, colors.gray);
    }

    // Manifest entrypoint
    if (bundle.hasFile('/manifest.webmanifest')) {
      entrypoints.manifest = '/manifest.webmanifest';
      log(`  Set entrypoint: manifest -> /manifest.webmanifest`, colors.gray);
    }

    // Set all entrypoints
    for (const [name, path] of Object.entries(entrypoints)) {
      bundle.setEntrypoint(name, path);
    }

    // Step 8: Validate bundle
    log('Validating bundle...', colors.blue);
    const validation = bundle.validate();
    if (!validation.valid) {
      log('Bundle validation warnings:', colors.yellow);
      validation.errors.forEach(error => {
        log(`    - ${error.message}`, colors.yellow);
      });
    } else {
      log('Bundle validation passed', colors.green);
    }

    // Step 9: Save bundle
    log('Saving bundle...', colors.blue);
    const bundleData = await bundle.toArrayBuffer();
    fs.writeFileSync(outputPath, Buffer.from(bundleData));

    // Step 10: Show summary
    const info = bundle.getBundleInfo();
    const sizeKB = (bundleData.byteLength / 1024).toFixed(2);
    const sizeMB = (bundleData.byteLength / 1024 / 1024).toFixed(2);

    log('', colors.reset);
    log('Bundle created successfully!', colors.green);
    log(`  Bundle: ${path.basename(outputPath)}`, colors.gray);
    log(`  Files: ${info.fileCount}`, colors.gray);
    log(`  Size: ${sizeKB} KB (${sizeMB} MB)`, colors.gray);
    log(`  Entrypoints: ${info.entrypoints.join(', ')}`, colors.gray);
    log(`  Location: ${outputPath}`, colors.gray);
    log('', colors.reset);
    log('Ready to load in Tonk Browser!', colors.blue);
  } catch (error: unknown) {
    log(`Bundling failed: ${error}`, colors.red);
    process.exit(1);
  }
}

// Run the bundler with command line argument
if (import.meta.url === `file://${process.argv[1]}`) {
  const appPath = process.argv[2];

  if (process.argv.includes('--help') || process.argv.includes('-h')) {
    console.log(`
Usage: bundle-tonk-app [path-to-app]

Bundle a Tonk app into a .tonk file

Options:
  [path-to-app]  Path to the Tonk app directory (defaults to current directory)
  --help, -h     Show this help message

Examples:
  bundle-tonk-app                    # Bundle app in current directory
  bundle-tonk-app ./my-app           # Bundle app at ./my-app
  bundle-tonk-app /path/to/app       # Bundle app at absolute path
    `);
    process.exit(0);
  }

  bundleTonkApp(appPath).catch(console.error);
}

export { bundleTonkApp };
