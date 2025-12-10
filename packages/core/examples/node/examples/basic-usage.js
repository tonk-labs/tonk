/**
 * Basic usage example for Tonk Core WASM bindings in Node.js
 *
 * This example demonstrates fundamental operations with Tonk Core:
 * - Creating sync engines
 * - VFS operations (files and directories)
 * - Bundle operations
 *
 * Run with: npm run example:basic
 */

// Import utilities
const path = require('path');

// Load the WASM module
let wasm;
try {
  wasm = require(path.resolve(__dirname, '../../../pkg-node/tonk_core.js'));
} catch (error) {
  console.error(
    '❌ Failed to load WASM module. Make sure to run "npm run build:wasm" first.'
  );
  console.error('Error:', error.message);
  process.exit(1);
}

async function main() {
  console.log('Tonk Core WASM Node.js Basic Usage Example\n');

  try {
    // Create a sync engine
    console.log('📦 Creating sync engine...');
    const engine = await wasm.create_sync_engine();
    console.log('HIT');
    const peerId = await engine.getPeerId();
    console.log(`✅ Sync engine created with peer ID: ${peerId}`);

    // Get VFS instance
    console.log('\n📁 Getting VFS instance...');
    const vfs = await engine.getVfs();
    console.log('✅ VFS instance obtained');

    // Create a file
    console.log('\n📝 Creating file /test/hello.txt...');
    await vfs.createFile('/test/hello.txt', 'Hello from Node.js!');
    console.log('✅ File created successfully');

    // Check if file exists
    const exists = await vfs.exists('/test/hello.txt');
    console.log(`File exists: ${exists}`);

    // Get file metadata
    const metadata = await vfs.getMetadata('/test/hello.txt');
    console.log('File metadata:', metadata);

    // Create a directory
    console.log('\nCreating directory /documents...');
    await vfs.createDirectory('/documents');
    console.log('Directory created');

    // Create multiple files
    console.log('\nCreating multiple files...');
    await vfs.createFile('/documents/file1.txt', 'Content of file 1');
    await vfs.createFile('/documents/file2.txt', 'Content of file 2');
    await vfs.createFile('/documents/file3.txt', 'Content of file 3');
    console.log('Files created');

    // List directory contents
    console.log('\nListing /documents directory...');
    const entries = await vfs.listDirectory('/documents');
    console.log('Directory contents:');
    entries.forEach(entry => {
      console.log(`  - ${entry.name} (${entry.type})`);
    });

    // Delete a file
    console.log('\nDeleting /documents/file2.txt...');
    const deleted = await vfs.deleteFile('/documents/file2.txt');
    console.log(`File deleted: ${deleted}`);

    // List directory again
    console.log('\nListing /documents directory after deletion...');
    const entriesAfter = await vfs.listDirectory('/documents');
    console.log('Directory contents:');
    entriesAfter.forEach(entry => {
      console.log(`  - ${entry.name} (${entry.type})`);
    });

    // WebSocket connection example (commented out as it requires a server)
    // console.log('\nConnecting to WebSocket server...');
    // await engine.connectWebsocket('ws://localhost:8081');
    // console.log('Connected to WebSocket server');
  } catch (error) {
    console.error('Error:', error);
  }
}

// Bundle operations example
async function bundleExample() {
  console.log('\n\n📦 Bundle Operations Example\n');

  try {
    // Create a new empty bundle
    console.log('📦 Creating a new bundle...');
    const bundle = wasm.create_bundle();
    console.log('✅ Bundle created successfully');

    // Add some data to the bundle
    console.log('\n📝 Adding data to bundle...');
    const configs = {
      'config/app.json': JSON.stringify({ name: 'MyApp', version: '1.0.0' }),
      'config/database.json': JSON.stringify({ host: 'localhost', port: 5432 }),
      'README.md': '# My Project\n\nThis is a sample project using Tonk Core.',
    };

    for (const [key, content] of Object.entries(configs)) {
      const data = new TextEncoder().encode(content);
      await bundle.put(key, data);
      console.log(`✅ Added: ${key}`);
    }

    // List all keys in the bundle
    console.log('\n📋 Listing bundle contents...');
    const keys = await bundle.listKeys();
    console.log('Bundle contains:');
    keys.forEach(key => console.log(`  - ${key}`));

    // Retrieve and display content
    console.log('\n📄 Reading config/app.json...');
    const appConfigData = await bundle.get('config/app.json');
    if (appConfigData) {
      const appConfig = new TextDecoder().decode(appConfigData);
      console.log('Content:', appConfig);
    }

    // Demonstrate prefix queries
    console.log('\n🔍 Finding config files...');
    const configFiles = await bundle.getPrefix('config/');
    console.log(`Found ${configFiles.length} config files:`);
    configFiles.forEach(({ key, value }) => {
      const content = new TextDecoder().decode(value);
      console.log(`  - ${key}: ${content.substring(0, 50)}...`);
    });

    // Demonstrate serialization
    console.log('\n💾 Serializing bundle...');
    const serialized = await bundle.toBytes();
    console.log(`✅ Bundle serialized to ${serialized.length} bytes`);

    // Create new bundle from serialized data
    console.log('\n📦 Creating bundle from serialized data...');
    const newBundle = wasm.create_bundle_from_bytes(serialized);
    const newKeys = await newBundle.listKeys();
    console.log(`✅ Restored bundle with ${newKeys.length} items`);

    // Delete a file to demonstrate deletion
    console.log('\n🗑 Deleting README.md...');
    const deleted = await bundle.delete('README.md');
    console.log(`✅ File deleted: ${deleted}`);

    const keysAfterDelete = await bundle.listKeys();
    console.log(`Bundle now has ${keysAfterDelete.length} files`);

    console.log('\n✅ Bundle operations completed successfully!');
  } catch (error) {
    console.error('❌ Bundle error:', error);
    throw error;
  }
}

// Performance demonstration
async function performanceDemo() {
  console.log('\n\n⚡ Performance Demo\n');

  try {
    const engine = await wasm.create_sync_engine();
    const vfs = await engine.getVfs();

    // Time file operations
    const start = Date.now();
    const fileCount = 100;

    console.log(`📝 Creating ${fileCount} files...`);
    for (let i = 0; i < fileCount; i++) {
      await vfs.createFile(`/perf/file${i}.txt`, `Content for file ${i}`);
    }

    const createTime = Date.now() - start;
    console.log(`✅ Created ${fileCount} files in ${createTime}ms`);
    console.log(
      `   Rate: ${((fileCount / createTime) * 1000).toFixed(0)} files/sec`
    );

    // Time directory listing
    const listStart = Date.now();
    const entries = await vfs.listDirectory('/perf');
    const listTime = Date.now() - listStart;

    console.log(`📋 Listed ${entries.length} files in ${listTime}ms`);
  } catch (error) {
    console.error('❌ Performance demo error:', error);
    throw error;
  }
}

// Main execution
async function runExamples() {
  console.log('🎯 Running all examples...\n');

  try {
    await main();
    await bundleExample();
    await performanceDemo();

    console.log('\n🎉 All examples completed successfully!');
  } catch (error) {
    console.error('\n💥 Fatal error:', error);
    process.exit(1);
  }
}

// Run all examples
runExamples();
