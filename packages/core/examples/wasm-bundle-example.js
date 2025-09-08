#!/usr/bin/env node

// Example demonstrating WASM bundle functionality in TonkCore
// This shows how to:
// 1. Create a TonkCore instance
// 2. Add data to it
// 3. Export to a bundle
// 4. Load from a bundle
// 5. Verify the data is preserved

import { WasmTonkCore, WasmBundle } from '../pkg-node/tonk_core.js';

async function main() {
  console.log('WASM Bundle Example');
  console.log('===================\n');

  // Step 1: Create a new TonkCore instance
  console.log('1. Creating new TonkCore instance...');
  const tonk = await new WasmTonkCore();
  console.log('✓ Created TonkCore');

  // Step 2: Get VFS and add some test data
  console.log('\n2. Adding test data...');
  const vfs = await tonk.getVfs();

  await vfs.createDirectory('/docs');
  await vfs.createFile(
    '/docs/readme.md',
    '# Welcome to TonkCore\n\nThis is a test document.'
  );
  await vfs.createFile(
    '/config.json',
    JSON.stringify({ version: '1.0', features: ['sync', 'bundle'] }, null, 2)
  );
  console.log('✓ Created test files and directories');

  // Step 3: Export to bundle
  console.log('\n3. Exporting to bundle...');
  const bundleBytes = await tonk.toBytes();
  console.log(`✓ Exported bundle (${bundleBytes.length} bytes)`);

  // Step 4: Load from bundle bytes
  console.log('\n4. Loading from bundle bytes...');
  const newTonk = await WasmTonkCore.fromBytes(bundleBytes);
  console.log('✓ Created new TonkCore from bundle');

  // Step 5: Verify data
  console.log('\n5. Verifying data...');
  const newVfs = await newTonk.getVfs();

  // Check if files exist
  const readmeExists = await newVfs.exists('/docs/readme.md');
  const configExists = await newVfs.exists('/config.json');

  console.log(`✓ /docs/readme.md exists: ${readmeExists}`);
  console.log(`✓ /config.json exists: ${configExists}`);

  // Read and display file contents
  const readmeContent = await newVfs.readFile('/docs/readme.md');
  const configContent = await newVfs.readFile('/config.json');

  console.log('\nFile contents:');
  console.log('README:', JSON.parse(readmeContent).content);
  console.log('CONFIG:', JSON.parse(configContent).content);

  // Step 6: Demonstrate WasmBundle usage
  console.log('\n6. Using WasmBundle...');
  const bundle = WasmBundle.fromBytes(bundleBytes);

  // List bundle keys
  const keys = await bundle.listKeys();
  console.log(`✓ Bundle contains ${keys.length} entries`);

  // Load TonkCore from WasmBundle
  const tonkFromBundle = await WasmTonkCore.fromBundle(bundle);
  const vfsFromBundle = await tonkFromBundle.getVfs();
  const verifyExists = await vfsFromBundle.exists('/docs/readme.md');
  console.log(`✓ Loaded from WasmBundle, data verified: ${verifyExists}`);

  console.log('\n✅ Bundle functionality test completed successfully!');
}

main().catch(err => {
  console.error('Error:', err);
  process.exit(1);
});

