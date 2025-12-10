import fs from 'fs';
import path from 'path';
import { generateWasmBase64 } from './encode-wasm.ts';

const WASM_FILES = [
  {
    input: './dist/tonk_core_bg.wasm',
    output: './src/generated/wasm-data.ts',
  },
];

async function main() {
  console.log('Embedding WASM files as base64...');

  for (const { input, output } of WASM_FILES) {
    const inputPath = path.resolve(input);
    const outputPath = path.resolve(output);

    const outputDir = path.dirname(outputPath);
    if (!fs.existsSync(outputDir)) {
      fs.mkdirSync(outputDir, { recursive: true });
    }

    generateWasmBase64(inputPath, outputPath);
  }

  console.log('All WASM files embedded successfully!');
}

main().catch(console.error);
