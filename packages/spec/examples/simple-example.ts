#!/usr/bin/env node

/**
 * Simple Bundle Example
 *
 * A shorter example showing basic bundle creation and analysis.
 * Run with: npx tsx examples/simple-example.ts
 */

import {
  ZipBundle,
  createBundleFromFiles,
  parseBundle,
  getBundleInfo,
  formatBytes,
  type Bundle,
} from '../src/index.js';

async function createSimpleBundle(): Promise<Bundle> {
  // Create some sample files
  const files = new Map<string, ArrayBuffer>();

  files.set(
    '/hello.html',
    new TextEncoder().encode(`
    <!DOCTYPE html>
    <html>
    <head><title>Hello Bundle</title></head>
    <body>
      <h1>Hello from Bundle!</h1>
      <p>This HTML file is packed in a bundle.</p>
    </body>
    </html>
  `).buffer
  );

  files.set(
    '/style.css',
    new TextEncoder().encode(`
    body { font-family: Arial, sans-serif; margin: 40px; }
    h1 { color: #333; }
  `).buffer
  );

  files.set(
    '/app.js',
    new TextEncoder().encode(`
    console.log('Bundle loaded successfully!');
    document.addEventListener('DOMContentLoaded', () => {
      console.log('DOM ready in bundled app');
    });
  `).buffer
  );

  // Create bundle
  const bundle = await createBundleFromFiles(files);

  // Set an entrypoint
  bundle.setEntrypoint('main', '/hello.html');

  return bundle;
}

async function main() {
  console.log('🚀 Simple Bundle Example\n');

  // Create bundle
  console.log('📦 Creating bundle...');
  const bundle = await createSimpleBundle();
  console.log('✅ Bundle created!');

  // Show basic info
  console.log('\n📊 Bundle Information:');
  const info = getBundleInfo(bundle);
  console.log(`   Files: ${info.fileCount}`);
  console.log(`   Size: ${formatBytes(info.totalSize)}`);
  console.log(`   Version: ${bundle.manifest.version}`);

  // List files
  console.log('\n📁 Files in bundle:');
  const files = bundle.listFiles();
  files.forEach(file => {
    console.log(
      `   ${file.path} (${formatBytes(file.size)}, ${file.contentType})`
    );
  });

  // Show entrypoints
  console.log('\n🎯 Entrypoints:');
  Object.entries(bundle.manifest.entrypoints).forEach(([name, path]) => {
    console.log(`   ${name}: ${path}`);
  });

  // Pack and unpack
  console.log('\n📦 Packing bundle...');
  const packed = await bundle.toArrayBuffer();
  console.log(`✅ Packed to ${formatBytes(packed.byteLength)}`);

  console.log('\n📂 Unpacking bundle...');
  const unpacked = await parseBundle(packed);
  console.log('✅ Unpacked successfully!');

  // Verify
  const unpackedInfo = getBundleInfo(unpacked);
  console.log(`\n🔍 Verification:`);
  console.log(
    `   Original files: ${info.fileCount}, Unpacked files: ${unpackedInfo.fileCount}`
  );
  console.log(
    `   Match: ${info.fileCount === unpackedInfo.fileCount ? '✅' : '❌'}`
  );

  // Read a file from unpacked bundle
  console.log('\n📄 Reading file from unpacked bundle:');
  const htmlData = await unpacked.getFileData('/hello.html');
  if (htmlData) {
    const content = new TextDecoder().decode(htmlData);
    console.log(`   Content preview: ${content.substring(0, 100)}...`);
  }

  console.log('\n🎉 Example completed successfully!');
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch(console.error);
}

export { main };
