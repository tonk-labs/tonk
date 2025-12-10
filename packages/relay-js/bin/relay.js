#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { getBinaryPath } from '../dist/binary.js';

const binary = getBinaryPath();
const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', err => {
  console.error('Failed to start relay:', err.message);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.exit(
      128 + (signal === 'SIGINT' ? 2 : signal === 'SIGTERM' ? 15 : 1)
    );
  }
  process.exit(code ?? 0);
});

// Forward signals to child
process.on('SIGINT', () => child.kill('SIGINT'));
process.on('SIGTERM', () => child.kill('SIGTERM'));
