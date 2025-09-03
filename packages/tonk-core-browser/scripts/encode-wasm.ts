import fs from 'fs';
import path from 'path';

export function generateWasmBase64(wasmFilePath: string, outputPath: string) {
  try {
    // Read the WASM file
    const wasmBuffer = fs.readFileSync(wasmFilePath);

    // Convert to base64
    const base64String = wasmBuffer.toString('base64');

    // Generate the JS/TS module
    const moduleContent = `// Auto-generated file - do not edit manually
// Generated from: ${path.basename(wasmFilePath)}
// Size: ${wasmBuffer.length} bytes

export const WASM_BASE64 = "${base64String}";
`;

    // Write to output file
    fs.writeFileSync(outputPath, moduleContent);

    console.log('Generated base64 WASM module:');
    console.log(`  Input: ${wasmFilePath} (${wasmBuffer.length} bytes)`);
    console.log(`  Output: ${outputPath}`);
  } catch (err) {
    console.error('Error generating base64 WASM:', err.message);
    process.exit(1);
  }
}

if (import.meta.main === true) {
  const wasmPath = process.argv[2];
  const outputPath = process.argv[3] || './src/wasm-base64.ts';

  if (!wasmPath) {
    console.error('Usage: tsx encode-wasm.ts <wasm-file> [output-file]');
    process.exit(1);
  }

  generateWasmBase64(wasmPath, outputPath);
}
