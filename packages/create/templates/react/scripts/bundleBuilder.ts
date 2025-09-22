import { TonkCore } from '@tonk/core';
import { readFileSync, writeFileSync, readdirSync, statSync, mkdirSync, existsSync } from 'fs';
import { join, relative, dirname } from 'path';

/**
 * Converts a Uint8Array to base64 string
 */
function uint8ArrayToBase64(uint8Array: Uint8Array): string {
  return Buffer.from(uint8Array).toString('base64');
}

/**
 * Converts a base64 string back to Uint8Array
 */
function base64ToUint8Array(base64: string): Uint8Array {
  return new Uint8Array(Buffer.from(base64, 'base64'));
}

/**
 * Recursively reads all files from a directory and returns them as an array
 * of objects containing the relative path and file data as Uint8Array
 */
function readDistFiles(distPath: string): Array<{ relativePath: string; fileData: Uint8Array }> {
  const files: Array<{ relativePath: string; fileData: Uint8Array }> = [];
  
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
        const relativePath = '/' + relative(basePath, fullPath).replace(/\\/g, '/');
        const fileData = new Uint8Array(readFileSync(fullPath));
        files.push({ relativePath, fileData });
      }
    }
  }
  
  walkDirectory(distPath, distPath);
  return files;
}

/**
 * Recursively gets all files from the VFS using a queue-based approach
 */
async function getAllFilesFromVfs(vfs: any, startPath: string = '/'): Promise<string[]> {
  const allFiles: string[] = [];
  const queue: string[] = [startPath];
  
  while (queue.length > 0) {
    const currentPath = queue.shift()!;
    
    try {
      const entries = await vfs.listDirectory(currentPath);
      
      for (const entry of entries) {
        const fullPath = currentPath === '/' ? `/${entry.name}` : `${currentPath}/${entry.name}`;
        
        if (entry.type === 'directory') {
          // Add directory to queue for processing
          queue.push(fullPath);
        } else if (entry.type === 'document') {
          // Add file to the list
          allFiles.push(fullPath);
        }
      }
    } catch (error) {
      // If we can't list a directory, skip it silently
      console.warn(`Could not list directory ${currentPath}:`, error);
    }
  }
  
  return allFiles;
}

/**
 * Creates a bundle from the dist/ folder
 */
async function createBundle(outputPath?: string) {
  try {
    console.log('Initializing TonkCore...');
    
    // Initialize TonkCore
    const tonk = await TonkCore.create();
    const vfs = await tonk.getVfs();
    
    // Get project name from current directory
    const projectName = process.cwd().split('/').pop() || 'tonk-project';
    
    console.log('Reading files from dist/ folder...');
    
    // Read all files from dist folder
    const distPath = join(process.cwd(), 'dist');
    
    if (!existsSync(distPath)) {
      throw new Error('dist/ folder not found. Please run "npm run build" first.');
    }
    
    const distFiles = readDistFiles(distPath);
    
    console.log(`Found ${distFiles.length} files to bundle`);
    
    // Store each file in the TonkCore VFS under /app
    for (const { relativePath, fileData } of distFiles) {
      const modifiedPath = '/app/' + `${projectName}/` + relativePath;
      console.log(`Adding file: ${modifiedPath}`);
      // Convert file data to base64 for VFS storage
      const base64Data = uint8ArrayToBase64(fileData);
      await vfs.createFile(modifiedPath, base64Data);
    }
    
    console.log('Creating bundle...');
    
    // Get the bundle bytes
    const bytes = await tonk.toBytes();
    
    // Save the bundle as a .tonk file
    const bundlePath = outputPath || join(process.cwd(), `${projectName}.tonk`);
    
    writeFileSync(bundlePath, bytes);
    
    console.log(`Bundle created successfully: ${bundlePath}`);
    console.log(`Bundle size: ${bytes.length} bytes`);
    
  } catch (error) {
    console.error('Error creating bundle:', error);
    process.exit(1);
  }
}

/**
 * Unpacks a bundle into a specified folder
 */
async function unpackBundle(bundlePath: string, outputDir: string) {
  try {
    console.log(`Unpacking bundle: ${bundlePath}`);
    
    if (!existsSync(bundlePath)) {
      throw new Error(`Bundle file not found: ${bundlePath}`);
    }
    
    // Read the bundle file
    const bundleData = readFileSync(bundlePath);
    
    // Initialize TonkCore from the bundle
    const tonk = await TonkCore.fromBytes(bundleData);
    const vfs = await tonk.getVfs();
    
    // Get all files from the VFS
    const files = await getAllFilesFromVfs(vfs, '/');
    
    console.log(`Found ${files.length} files in bundle`);
    
    // Create output directory if it doesn't exist
    if (!existsSync(outputDir)) {
      mkdirSync(outputDir, { recursive: true });
    }
    
    // Extract each file
    for (const filePath of files) {
      console.log(`Extracting: ${filePath}`);
      
      const raw = await vfs.readFile(filePath);
      
      // Convert base64 back to bytes for file writing
      const fileData = base64ToUint8Array(raw);
      
      // Remove /app prefix if it exists for cleaner output structure
      const cleanPath = filePath.startsWith('/app/') ? filePath.substring(4) : filePath;
      const outputPath = join(outputDir, cleanPath);
      
      // Create directory structure if needed
      const fileDir = dirname(outputPath);
      if (!existsSync(fileDir)) {
        mkdirSync(fileDir, { recursive: true });
      }
      
      // Write the file
      writeFileSync(outputPath, fileData);
    }
    
    console.log(`Bundle unpacked successfully to: ${outputDir}`);
    
  } catch (error) {
    console.error('Error unpacking bundle:', error);
    process.exit(1);
  }
}

/**
 * Lists directories and files in a bundle
 */
async function listBundle(bundlePath: string) {
  try {
    console.log(`Listing contents of bundle: ${bundlePath}`);
    
    if (!existsSync(bundlePath)) {
      throw new Error(`Bundle file not found: ${bundlePath}`);
    }
    
    // Read the bundle file
    const bundleData = readFileSync(bundlePath);
    
    // Initialize TonkCore from the bundle
    const tonk = await TonkCore.fromBytes(bundleData);
    const vfs = await tonk.getVfs();
    
    // Get all files from the VFS
    const files = await getAllFilesFromVfs(vfs, '/');
    
    console.log(`\nBundle contains ${files.length} files:\n`);
    
    // Group files by directory for better visualization
    const directories = new Map<string, string[]>();
    
    for (const filePath of files) {
      const dir = dirname(filePath);
      if (!directories.has(dir)) {
        directories.set(dir, []);
      }
      directories.get(dir)!.push(filePath);
    }
    
    // Sort directories and display
    const sortedDirs = Array.from(directories.keys()).sort();
    
    for (const dir of sortedDirs) {
      console.log(`📁 ${dir}/`);
      const dirFiles = directories.get(dir)!.sort();
      for (const file of dirFiles) {
        const fileName = file.substring(file.lastIndexOf('/') + 1);
        console.log(`  📄 ${fileName}`);
      }
      console.log();
    }
    
  } catch (error) {
    console.error('Error listing bundle:', error);
    process.exit(1);
  }
}

/**
 * Displays help information
 */
function showHelp() {
  console.log(`
Bundle Builder - Create, unpack, and inspect Tonk bundles

Usage:
  npm run bundle-builder [command] [options]

Commands:
  create [output]     Create a bundle from dist/ folder
                      Optional: specify output path (default: test-wasm.tonk)
  
  unpack <bundle> <output-dir>
                      Unpack a bundle into the specified directory
  
  list <bundle>       List all directories and files in a bundle

Examples:
  npm run bundle-builder create
  npm run bundle-builder create my-app.tonk
  npm run bundle-builder unpack test-wasm.tonk ./unpacked
  npm run bundle-builder list test-wasm.tonk

Options:
  --help, -h         Show this help message
`);
}

/**
 * Main function to handle command-line arguments
 */
async function main() {
  const args = process.argv.slice(2);
  
  if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
    showHelp();
    return;
  }
  
  const command = args[0];
  
  switch (command) {
    case 'create':
      const outputPath = args[1];
      await createBundle(outputPath);
      break;
      
    case 'unpack':
      if (args.length < 3) {
        console.error('Error: unpack command requires bundle path and output directory');
        console.log('Usage: npm run bundle-builder unpack <bundle> <output-dir>');
        process.exit(1);
      }
      await unpackBundle(args[1], args[2]);
      break;
      
    case 'list':
      if (args.length < 2) {
        console.error('Error: list command requires bundle path');
        console.log('Usage: npm run bundle-builder list <bundle>');
        process.exit(1);
      }
      await listBundle(args[1]);
      break;
      
    default:
      console.error(`Error: Unknown command '${command}'`);
      showHelp();
      process.exit(1);
  }
}

// Run the main function
main();
