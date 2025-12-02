import { WASM_BASE64 } from '@tonk/core';

const wasmBytes = Buffer.from(WASM_BASE64, 'base64');
process.stdout.write(wasmBytes);
