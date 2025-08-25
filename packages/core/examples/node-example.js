// Example of using Tonk Core WASM bindings in Node.js

// Note: This example assumes you've built the WASM module with:
// wasm-pack build --target nodejs --out-dir pkg-node

const {
  create_sync_engine,
  create_sync_engine_with_peer_id,
  create_bundle_from_bytes,
} = require('../pkg-node/tonk_core.js');

async function main() {
  console.log('Tonk Core WASM Node.js Example\n');

  try {
    // Create a sync engine
    console.log('Creating sync engine...');
    const engine = await create_sync_engine();
    const peerId = await engine.getPeerId();
    console.log(`Sync engine created with peer ID: ${peerId}`);

    // Get VFS instance
    console.log('\nGetting VFS instance...');
    const vfs = await engine.getVfs();
    console.log('VFS instance obtained');

    // Create a file
    console.log('\nCreating file /test/hello.txt...');
    await vfs.createFile('/test/hello.txt', 'Hello from Node.js!');
    console.log('File created successfully');

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
    // await engine.connectWebsocket('ws://localhost:8080');
    // console.log('Connected to WebSocket server');
  } catch (error) {
    console.error('Error:', error);
  }
}

// Bundle operations example
async function bundleExample() {
  console.log('\n\nBundle Operations Example\n');

  try {
    // In a real scenario, you would load bundle data from a file
    // For this example, we'll show the API usage

    console.log('Note: Bundle operations require an existing bundle file.');
    console.log('The API would be used like this:');
    console.log(`
// Load bundle from file
const fs = require('fs');
const bundleData = fs.readFileSync('path/to/bundle.zip');
const bundle = create_bundle_from_bytes(new Uint8Array(bundleData));

// Put a value
const key = 'config/settings.json';
const value = new TextEncoder().encode(JSON.stringify({ theme: 'dark' }));
await bundle.put(key, value);

// Get a value
const data = await bundle.get(key);
const content = new TextDecoder().decode(data);
console.log('Retrieved:', content);

// List all keys
const keys = await bundle.listKeys();
console.log('Bundle keys:', keys);

// Delete a key
await bundle.delete(key);
`);
  } catch (error) {
    console.error('Bundle error:', error);
  }
}

// Run the examples
main()
  .then(() => bundleExample())
  .then(() => {
    console.log('\nExample completed!');
  })
  .catch(error => {
    console.error('Fatal error:', error);
    process.exit(1);
  });
