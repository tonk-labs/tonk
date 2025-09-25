#!/usr/bin/env tsx

import { execSync } from 'child_process';
import {
  existsSync,
  rmSync,
  mkdirSync,
  readdirSync,
  statSync,
  readFileSync,
} from 'fs';
import { join, relative } from 'path';
import chalk from 'chalk';

/**
 * Test script for bundleBuilder.ts
 * This script tests all three main functions: create, list, and unpack
 */

const TEST_DIR = join(process.cwd(), 'test-bundle-output');
const TEST_BUNDLE_PATH = join(TEST_DIR, 'test-bundle.tonk');
const TEST_UNPACK_DIR = join(TEST_DIR, 'unpacked');
const BUNDLE_BUILDER_SCRIPT = join(
  process.cwd(),
  'scripts',
  'bundleBuilder.ts'
);

interface TestResult {
  name: string;
  passed: boolean;
  error?: string;
  duration: number;
}

interface FileInfo {
  relativePath: string;
  fullPath: string;
  size: number;
  content: Buffer;
}

const testResults: TestResult[] = [];
let originalDistFiles: FileInfo[] = [];

/**
 * Recursively reads all files from a directory and returns detailed file information
 */
function readAllFiles(distPath: string): FileInfo[] {
  const files: FileInfo[] = [];

  function walkDirectory(currentPath: string, basePath: string) {
    const items = readdirSync(currentPath);

    for (const item of items) {
      const fullPath = join(currentPath, item);
      const stat = statSync(fullPath);

      if (stat.isDirectory()) {
        // Recursively walk subdirectories
        walkDirectory(fullPath, basePath);
      } else if (stat.isFile()) {
        // Read file and store with relative path
        const relativePath =
          '/' + relative(basePath, fullPath).replace(/\\/g, '/');
        const content = readFileSync(fullPath);
        files.push({
          relativePath,
          fullPath,
          size: stat.size,
          content,
        });
      }
    }
  }

  walkDirectory(distPath, distPath);
  return files;
}

/**
 * Compare two files for exact content match
 */
function compareFiles(file1: Buffer, file2: Buffer): boolean {
  return Buffer.compare(file1, file2) === 0;
}

/**
 * Find a file in the unpacked directory that corresponds to an original dist file
 */
function findUnpackedFile(
  originalFile: FileInfo,
  unpackedDir: string,
  projectName: string
): string | null {
  // The bundleBuilder stores files under /app/{projectName}/ in the VFS
  // So we need to look for: unpackedDir/app/{projectName}/{originalFile.relativePath}
  const expectedPath = join(
    unpackedDir,
    'app',
    projectName,
    originalFile.relativePath
  );

  if (existsSync(expectedPath)) {
    return expectedPath;
  }

  // Fallback: try without the leading slash
  const alternativePath = join(
    unpackedDir,
    'app',
    projectName,
    originalFile.relativePath.substring(1)
  );
  if (existsSync(alternativePath)) {
    return alternativePath;
  }

  return null;
}

/**
 * Utility function to run a test and capture results
 */
async function runTest(
  testName: string,
  testFn: () => Promise<void>
): Promise<void> {
  const startTime = Date.now();
  console.log(chalk.blue(`\n🧪 Running test: ${testName}`));

  try {
    await testFn();
    const duration = Date.now() - startTime;
    testResults.push({ name: testName, passed: true, duration });
    console.log(chalk.green(`✅ ${testName} passed (${duration}ms)`));
  } catch (error) {
    const duration = Date.now() - startTime;
    const errorMessage = error instanceof Error ? error.message : String(error);
    testResults.push({
      name: testName,
      passed: false,
      error: errorMessage,
      duration,
    });
    console.log(
      chalk.red(`❌ ${testName} failed (${duration}ms): ${errorMessage}`)
    );
  }
}

/**
 * Setup test environment
 */
async function setupTestEnvironment(): Promise<void> {
  console.log(chalk.yellow('🔧 Setting up test environment...'));

  // Clean up any existing test directory
  if (existsSync(TEST_DIR)) {
    rmSync(TEST_DIR, { recursive: true, force: true });
  }

  // Create test directory
  mkdirSync(TEST_DIR, { recursive: true });

  // Verify dist folder exists
  const distPath = join(process.cwd(), 'dist');
  if (!existsSync(distPath)) {
    throw new Error(
      'dist/ folder not found. Please run "npm run build" first.'
    );
  }

  // Read all files from dist directory
  console.log('📁 Reading all files from dist directory...');
  originalDistFiles = readAllFiles(distPath);
  console.log(`Found ${originalDistFiles.length} files in dist directory`);

  // Log all files for reference
  console.log('📄 Files to be bundled:');
  originalDistFiles.forEach(file => {
    console.log(`  ${file.relativePath} (${file.size} bytes)`);
  });

  console.log(chalk.green('✅ Test environment setup complete'));
}

/**
 * Test bundle creation
 */
async function testCreateBundle(): Promise<void> {
  console.log('Creating bundle...');

  // Run the bundle creation command
  const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" create "${TEST_BUNDLE_PATH}"`;
  execSync(command, { stdio: 'pipe' });

  // Verify bundle was created
  if (!existsSync(TEST_BUNDLE_PATH)) {
    throw new Error('Bundle file was not created');
  }

  // Verify bundle has content
  const stats = statSync(TEST_BUNDLE_PATH);
  if (stats.size === 0) {
    throw new Error('Bundle file is empty');
  }

  console.log(
    `Bundle created successfully: ${TEST_BUNDLE_PATH} (${stats.size} bytes)`
  );
}

/**
 * Test bundle listing
 */
async function testListBundle(): Promise<void> {
  console.log('Listing bundle contents...');

  if (!existsSync(TEST_BUNDLE_PATH)) {
    throw new Error(
      'Test bundle does not exist. Create bundle test must run first.'
    );
  }

  // Run the list command and capture output
  const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" list "${TEST_BUNDLE_PATH}"`;
  const output = execSync(command, { encoding: 'utf-8' });

  // Verify output contains expected information
  if (!output.includes('Bundle contains')) {
    throw new Error(
      'List output does not contain expected "Bundle contains" text'
    );
  }

  if (!output.includes('files:')) {
    throw new Error('List output does not contain expected "files:" text');
  }

  // Check that all original files are mentioned in the bundle listing
  const missingFiles: string[] = [];

  for (const file of originalDistFiles) {
    // Extract just the filename for checking (since list shows filenames)
    const filename = file.relativePath.split('/').pop() || '';
    if (filename && !output.includes(filename)) {
      missingFiles.push(file.relativePath);
    }
  }

  if (missingFiles.length > 0) {
    throw new Error(
      `Bundle listing is missing ${missingFiles.length} files: ${missingFiles.join(', ')}`
    );
  }

  // Extract the file count from the output
  const fileCountMatch = output.match(/Bundle contains (\d+) files/);
  const bundleFileCount = fileCountMatch ? parseInt(fileCountMatch[1]) : 0;

  console.log(
    `Bundle listing successful. Bundle contains ${bundleFileCount} files, expected ${originalDistFiles.length} files.`
  );

  // Verify file count matches (allowing for some flexibility in case of additional metadata files)
  if (bundleFileCount < originalDistFiles.length) {
    throw new Error(
      `Bundle contains fewer files (${bundleFileCount}) than expected (${originalDistFiles.length})`
    );
  }
}

/**
 * Test bundle unpacking
 */
async function testUnpackBundle(): Promise<void> {
  console.log('Unpacking bundle...');

  if (!existsSync(TEST_BUNDLE_PATH)) {
    throw new Error(
      'Test bundle does not exist. Create bundle test must run first.'
    );
  }

  // Run the unpack command
  const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" unpack "${TEST_BUNDLE_PATH}" "${TEST_UNPACK_DIR}"`;
  execSync(command, { stdio: 'pipe' });

  // Verify unpack directory was created
  if (!existsSync(TEST_UNPACK_DIR)) {
    throw new Error('Unpack directory was not created');
  }

  // Count files in unpacked directory
  const unpackedFiles = countFilesRecursively(TEST_UNPACK_DIR);
  if (unpackedFiles === 0) {
    throw new Error('No files were unpacked');
  }

  // Get project name for path construction
  const projectName = process.cwd().split('/').pop() || 'tonk-project';

  // Verify all original files are present and have matching content
  const missingFiles: string[] = [];
  const contentMismatches: string[] = [];
  const sizeMatches: {
    file: string;
    originalSize: number;
    unpackedSize: number;
  }[] = [];

  console.log('🔍 Verifying all files are present and content matches...');

  for (const originalFile of originalDistFiles) {
    const unpackedFilePath = findUnpackedFile(
      originalFile,
      TEST_UNPACK_DIR,
      projectName
    );

    if (!unpackedFilePath) {
      missingFiles.push(originalFile.relativePath);
      continue;
    }

    // Read the unpacked file
    const unpackedContent = readFileSync(unpackedFilePath);
    const unpackedStat = statSync(unpackedFilePath);

    // Compare file sizes
    if (unpackedStat.size !== originalFile.size) {
      sizeMatches.push({
        file: originalFile.relativePath,
        originalSize: originalFile.size,
        unpackedSize: unpackedStat.size,
      });
    }

    // Compare file content
    if (!compareFiles(originalFile.content, unpackedContent)) {
      contentMismatches.push(originalFile.relativePath);
    }

    console.log(`✓ ${originalFile.relativePath} (${originalFile.size} bytes)`);
  }

  // Report any issues
  if (missingFiles.length > 0) {
    console.log('📁 Unpacked directory structure:');
    const contents = readdirSync(TEST_UNPACK_DIR);
    console.log('Top level:', contents);
    if (contents.includes('app')) {
      const appContents = readdirSync(join(TEST_UNPACK_DIR, 'app'));
      console.log('App level:', appContents);
      if (appContents.includes(projectName)) {
        const projectContents = readdirSync(
          join(TEST_UNPACK_DIR, 'app', projectName)
        );
        console.log(`Project level (${projectName}):`, projectContents);
      }
    }

    throw new Error(
      `${missingFiles.length} files are missing from unpacked bundle: ${missingFiles.join(', ')}`
    );
  }

  if (sizeMatches.length > 0) {
    console.log('⚠️  Size mismatches detected:');
    sizeMatches.forEach(({ file, originalSize, unpackedSize }) => {
      console.log(
        `  ${file}: original ${originalSize} bytes, unpacked ${unpackedSize} bytes`
      );
    });
    throw new Error(`${sizeMatches.length} files have size mismatches`);
  }

  if (contentMismatches.length > 0) {
    throw new Error(
      `${contentMismatches.length} files have content mismatches: ${contentMismatches.join(', ')}`
    );
  }

  console.log(
    `✅ Bundle unpacked successfully. All ${originalDistFiles.length} files verified with matching content.`
  );
}

/**
 * Test error handling
 */
async function testErrorHandling(): Promise<void> {
  console.log('Testing error handling...');

  // Test with non-existent bundle file
  try {
    const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" list "/non/existent/bundle.tonk"`;
    execSync(command, { stdio: 'pipe' });
    throw new Error(
      'Expected error for non-existent bundle file, but command succeeded'
    );
  } catch (error) {
    // This should fail, which is expected
    if (error instanceof Error && error.message.includes('Expected error')) {
      throw error; // Re-throw if it's our custom error
    }
    // Otherwise, this is the expected failure
  }

  // Test with invalid command
  try {
    const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" invalid-command`;
    execSync(command, { stdio: 'pipe' });
    throw new Error(
      'Expected error for invalid command, but command succeeded'
    );
  } catch (error) {
    // This should fail, which is expected
    if (error instanceof Error && error.message.includes('Expected error')) {
      throw error; // Re-throw if it's our custom error
    }
    // Otherwise, this is the expected failure
  }

  console.log('Error handling tests passed');
}

/**
 * Test help command
 */
async function testHelpCommand(): Promise<void> {
  console.log('Testing help command...');

  const command = `npx tsx "${BUNDLE_BUILDER_SCRIPT}" --help`;
  const output = execSync(command, { encoding: 'utf-8' });

  // Verify help output contains expected information
  if (!output.includes('Bundle Builder')) {
    throw new Error('Help output does not contain "Bundle Builder" title');
  }

  if (!output.includes('create')) {
    throw new Error('Help output does not contain "create" command');
  }

  if (!output.includes('unpack')) {
    throw new Error('Help output does not contain "unpack" command');
  }

  if (!output.includes('list')) {
    throw new Error('Help output does not contain "list" command');
  }

  console.log('Help command test passed');
}

/**
 * Utility function to count files recursively
 */
function countFilesRecursively(dir: string): number {
  let count = 0;

  try {
    const items = readdirSync(dir);

    for (const item of items) {
      const fullPath = join(dir, item);
      const stats = statSync(fullPath);

      if (stats.isDirectory()) {
        count += countFilesRecursively(fullPath);
      } else if (stats.isFile()) {
        count++;
      }
    }
  } catch (error) {
    console.warn(`Could not read directory ${dir}:`, error);
  }

  return count;
}

/**
 * Cleanup test environment
 */
async function cleanupTestEnvironment(): Promise<void> {
  console.log(chalk.yellow('\n🧹 Cleaning up test environment...'));

  if (existsSync(TEST_DIR)) {
    rmSync(TEST_DIR, { recursive: true, force: true });
  }

  console.log(chalk.green('✅ Test environment cleaned up'));
}

/**
 * Print test results summary
 */
function printTestSummary(): void {
  console.log(chalk.blue('\n📊 Test Results Summary'));
  console.log(chalk.blue('========================'));

  const passedTests = testResults.filter(t => t.passed);
  const failedTests = testResults.filter(t => !t.passed);

  console.log(chalk.green(`✅ Passed: ${passedTests.length}`));
  console.log(chalk.red(`❌ Failed: ${failedTests.length}`));
  console.log(chalk.blue(`📈 Total: ${testResults.length}`));

  if (failedTests.length > 0) {
    console.log(chalk.red('\nFailed Tests:'));
    failedTests.forEach(test => {
      console.log(chalk.red(`  • ${test.name}: ${test.error}`));
    });
  }

  const totalDuration = testResults.reduce(
    (sum, test) => sum + test.duration,
    0
  );
  console.log(chalk.blue(`⏱️  Total Duration: ${totalDuration}ms`));

  if (failedTests.length === 0) {
    console.log(chalk.green('\n🎉 All tests passed!'));
  } else {
    console.log(chalk.red(`\n💥 ${failedTests.length} test(s) failed.`));
    process.exit(1);
  }
}

/**
 * Main test function
 */
async function main(): Promise<void> {
  console.log(chalk.blue('🚀 Starting Bundle Builder Tests'));
  console.log(chalk.blue('=================================='));

  try {
    // Setup
    await runTest('Setup Test Environment', setupTestEnvironment);

    // Core functionality tests
    await runTest('Create Bundle', testCreateBundle);
    await runTest('List Bundle', testListBundle);
    await runTest('Unpack Bundle', testUnpackBundle);

    // Additional tests
    await runTest('Error Handling', testErrorHandling);
    await runTest('Help Command', testHelpCommand);
  } finally {
    // Always cleanup, even if tests fail
    await runTest('Cleanup Test Environment', cleanupTestEnvironment);

    // Print summary
    printTestSummary();
  }
}

// Handle uncaught errors
process.on('uncaughtException', error => {
  console.error(chalk.red('Uncaught Exception:'), error);
  process.exit(1);
});

process.on('unhandledRejection', (reason, promise) => {
  console.error(
    chalk.red('Unhandled Rejection at:'),
    promise,
    'reason:',
    reason
  );
  process.exit(1);
});

// Run the tests
main().catch(error => {
  console.error(chalk.red('Test execution failed:'), error);
  process.exit(1);
});
